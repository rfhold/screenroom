use crate::{records, validation};
use axum::http::StatusCode;
use hmac::{Hmac, Mac};
use screenroom_backend::{Uuid, playback::secrets::Capability};
use screenroom_catalog_adapter::jellyfin::playback as provider;
use sha2::Sha256;
use std::collections::HashMap;

const RESOURCE_LIMIT: usize = 8192;
const CACHE_LIMIT: usize = 2 * 1024 * 1024;
const DOMAIN: &[u8] = b"screenroom-playback-child\0v1\0";

pub(crate) struct SigningKey(Hmac<Sha256>);

#[derive(Clone, Copy)]
pub(crate) struct Context {
    pub session: Uuid,
    pub revision: u32,
    pub anchor: Uuid,
}

impl SigningKey {
    pub fn generate() -> Result<Self, StatusCode> {
        let key = Capability::generate().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        Hmac::new_from_slice(key.expose().as_bytes())
            .map(Self)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
    }

    fn mac(&self, context: Context, child: Uuid) -> Hmac<Sha256> {
        let mut mac = self.0.clone();
        mac.update(DOMAIN);
        mac.update(context.session.as_bytes());
        mac.update(&context.revision.to_be_bytes());
        mac.update(context.anchor.as_bytes());
        mac.update(child.as_bytes());
        mac
    }

    fn sign(&self, context: Context, child: Uuid) -> String {
        self.mac(context, child)
            .finalize()
            .into_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn verify(&self, context: Context, child: Uuid, signature: &[u8; 32]) -> bool {
        self.mac(context, child).verify_slice(signature).is_ok()
    }

    fn path(&self, context: Context, child: Uuid, capability: &str) -> String {
        format!(
            "{}&anchor={}&sig={}",
            records::path(context.session, context.revision, child, capability),
            context.anchor,
            self.sign(context, child),
        )
    }
}

pub(crate) struct ResourceQuery {
    pub capability: String,
    child: Option<(Uuid, [u8; 32])>,
}

impl ResourceQuery {
    pub fn parse(query: Option<&str>) -> Result<Self, StatusCode> {
        let query = query
            .filter(|s| s.len() <= 256)
            .ok_or(StatusCode::NOT_FOUND)?;
        let mut capability = None;
        let mut anchor = None;
        let mut signature = None;
        for field in query.split('&') {
            let (name, value) = field.split_once('=').ok_or(StatusCode::NOT_FOUND)?;
            match name {
                "cap" if capability.is_none() => {
                    validation::capability(value).map_err(|_| StatusCode::NOT_FOUND)?;
                    capability = Some(value.to_owned());
                }
                "anchor" if anchor.is_none() => {
                    anchor = Some(validation::uuid(value).map_err(|_| StatusCode::NOT_FOUND)?);
                }
                "sig" if signature.is_none() => {
                    validation::capability(value).map_err(|_| StatusCode::NOT_FOUND)?;
                    let mut bytes = [0; 32];
                    for (byte, pair) in bytes.iter_mut().zip(value.as_bytes().as_chunks::<2>().0) {
                        let hex = |n: u8| if n <= b'9' { n - b'0' } else { n - b'a' + 10 };
                        *byte = hex(pair[0]) * 16 + hex(pair[1]);
                    }
                    signature = Some(bytes);
                }
                _ => return Err(StatusCode::NOT_FOUND),
            }
        }
        let child = match (anchor, signature) {
            (None, None) => None,
            (Some(anchor), Some(signature)) => Some((anchor, signature)),
            _ => return Err(StatusCode::NOT_FOUND),
        };
        Ok(Self {
            capability: capability.ok_or(StatusCode::NOT_FOUND)?,
            child,
        })
    }
}

struct Entry {
    reference: provider::ResourceRef,
    anchor: Uuid,
    durable: bool,
}

pub(crate) struct ResourceGraph {
    entries: HashMap<Uuid, Entry>,
    children: HashMap<(Uuid, provider::ResourceRef), Uuid>,
}

impl ResourceGraph {
    pub fn new(roots: HashMap<Uuid, provider::ResourceRef>) -> Result<Self, StatusCode> {
        if roots.len() > RESOURCE_LIMIT {
            return Err(StatusCode::BAD_GATEWAY);
        }
        Ok(Self {
            entries: roots
                .into_iter()
                .map(|(id, reference)| {
                    (
                        id,
                        Entry {
                            reference,
                            anchor: id,
                            durable: true,
                        },
                    )
                })
                .collect(),
            children: HashMap::new(),
        })
    }

    pub fn durable_ids(&self) -> impl Iterator<Item = &Uuid> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.durable)
            .map(|(id, _)| id)
    }

    pub fn resolve(
        &self,
        key: &SigningKey,
        session: Uuid,
        revision: u32,
        resource: Uuid,
        query: &ResourceQuery,
    ) -> Result<(Uuid, provider::ResourceRef), StatusCode> {
        let entry = self.entries.get(&resource).ok_or(StatusCode::NOT_FOUND)?;
        match query.child {
            None if entry.durable => {}
            Some((anchor, signature))
                if !entry.durable
                    && anchor == entry.anchor
                    && self.entries.get(&anchor).is_some_and(|root| root.durable)
                    && key.verify(
                        Context {
                            session,
                            revision,
                            anchor,
                        },
                        resource,
                        &signature,
                    ) => {}
            _ => return Err(StatusCode::NOT_FOUND),
        }
        Ok((entry.anchor, entry.reference.clone()))
    }
}

pub(crate) struct PlaylistDraft<'a> {
    graph: &'a ResourceGraph,
    key: &'a SigningKey,
    context: Context,
    capability: &'a str,
    pending: HashMap<(Uuid, provider::ResourceRef), Uuid>,
}

impl<'a> PlaylistDraft<'a> {
    pub fn new(
        graph: &'a ResourceGraph,
        key: &'a SigningKey,
        context: Context,
        capability: &'a str,
    ) -> Self {
        Self {
            graph,
            key,
            context,
            capability,
            pending: HashMap::new(),
        }
    }

    pub fn map(&mut self, reference: provider::ResourceRef) -> provider::Result<String> {
        let identity = (self.context.anchor, reference);
        let id = if let Some(id) = self
            .graph
            .children
            .get(&identity)
            .or(self.pending.get(&identity))
        {
            *id
        } else {
            if self.graph.entries.len() + self.pending.len() >= RESOURCE_LIMIT {
                return Err(provider::PlaybackError {
                    failure: provider::Failure::Bounds,
                    effect: provider::Effect::NotDispatched,
                    prior_effect: None,
                });
            }
            let id = Uuid::new_v4();
            self.pending.insert(identity, id);
            id
        };
        Ok(self.key.path(self.context, id, self.capability))
    }

    pub fn finish(self, rewritten: String, budget: provider::PlaylistBudget) -> PendingPlaylist {
        PendingPlaylist {
            children: self.pending,
            rewritten,
            budget,
        }
    }
}

pub(crate) struct PendingPlaylist {
    children: HashMap<(Uuid, provider::ResourceRef), Uuid>,
    rewritten: String,
    budget: provider::PlaylistBudget,
}

impl PendingPlaylist {
    pub fn children(&self) -> usize {
        self.children.len()
    }
}

#[derive(Default)]
pub(crate) struct PlaylistState {
    pub budget: provider::PlaylistBudget,
    pub rewritten: HashMap<Uuid, String>,
    bytes: usize,
}

impl PlaylistState {
    // Called under both graph-write and playlist locks after the final durable fence.
    pub fn publish(
        &mut self,
        graph: &mut ResourceGraph,
        resource: Uuid,
        pending: PendingPlaylist,
    ) -> Result<(), StatusCode> {
        let bytes = self
            .bytes
            .checked_add(pending.rewritten.len())
            .filter(|total| *total <= CACHE_LIMIT)
            .ok_or(StatusCode::BAD_GATEWAY)?;
        if graph.entries.len() + pending.children.len() > RESOURCE_LIMIT
            || !graph
                .entries
                .get(&resource)
                .is_some_and(|entry| entry.reference.kind() == provider::ResourceKind::Playlist)
            || self.rewritten.contains_key(&resource)
            || pending.children.iter().any(|((anchor, reference), id)| {
                graph.entries.contains_key(id)
                    || graph.children.contains_key(&(*anchor, reference.clone()))
                    || !graph.entries.get(anchor).is_some_and(|root| root.durable)
            })
        {
            return Err(StatusCode::BAD_GATEWAY);
        }
        for ((anchor, reference), id) in pending.children {
            graph.entries.insert(
                id,
                Entry {
                    reference: reference.clone(),
                    anchor,
                    durable: false,
                },
            );
            graph.children.insert((anchor, reference), id);
        }
        self.rewritten.insert(resource, pending.rewritten);
        self.bytes = bytes;
        self.budget = pending.budget;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
