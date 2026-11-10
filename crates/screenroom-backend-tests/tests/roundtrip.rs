use screenroom_client_core::{Client, ClientConfig, Page, SourceLibraryScope};
use std::time::Duration;
mod support;

fn scope() -> SourceLibraryScope {
    SourceLibraryScope::default()
}

#[tokio::test]
#[ignore = "requires scripts/backend.sh up and explicit SCREENROOM_API_ENDPOINT"]
async fn persistent_native_core_roundtrip() {
    tokio::time::timeout(Duration::from_secs(120), roundtrip())
        .await
        .expect("whole roundtrip deadline");
}

async fn roundtrip() {
    let started = std::time::Instant::now();
    let endpoint = std::env::var("SCREENROOM_API_ENDPOINT").expect("explicit endpoint");
    let client = Client::new(ClientConfig {
        endpoint,
        user_id: "00000000-0000-0000-0000-000000000001".into(),
        timeout_ms: 15000,
    })
    .unwrap();
    assert_eq!(
        client
            .clone()
            .get_current_user()
            .await
            .unwrap()
            .display_name,
        "Alice"
    );
    let home = client.clone().get_home(scope()).await.unwrap();
    assert!(
        home.iter()
            .flat_map(|row| &row.items)
            .any(|item| item.progress_percent.is_some())
    );
    let item = client
        .clone()
        .get_item("big-buck-bunny".into(), scope())
        .await
        .unwrap();
    assert_eq!(item.title, "Big Buck Bunny");
    assert!(item.title_rights.as_ref().unwrap().label.contains("CC BY"));
    let image = item.artwork.unwrap();
    let bytes = client
        .clone()
        .load_artwork(image.url.clone())
        .await
        .unwrap();
    assert_eq!(
        bytes,
        include_bytes!("../../../fixtures/catalog/artwork/big-buck-bunny.jpg")
    );
    assert_eq!(bytes, client.clone().load_artwork(image.url).await.unwrap());
    let artwork = support::pages(
        |token| {
            let client = client.clone();
            async move {
                let page = client
                    .list_artwork(scope(), Page { size: 2, token })
                    .await
                    .map_err(|error| format!("{error:?}"))?;
                Ok((page.artwork, page.next_page_token))
            }
        },
        512,
        10000,
        Duration::from_secs(120),
    )
    .await
    .unwrap();
    let mut ids = std::collections::BTreeSet::new();
    for image in artwork {
        assert!(ids.insert(image.id));
        assert!(!image.license.is_empty());
        let filename = match image.label.as_str() {
            "His Girl Friday theatrical poster (1939)" => "his-girl-friday.jpg",
            "Big Buck Bunny poster" => "big-buck-bunny.jpg",
            "Sintel poster" => "sintel.jpg",
            "Tears of Steel poster" => "tears-of-steel.png",
            "Sacha Goedegebure portrait" => "sacha.jpg",
            "Andy Goralczyk portrait" => "andy.jpg",
            "Jan Morgenstern portrait" => "jan.jpg",
            "Ton Roosendaal portrait" => "ton.jpg",
            "Duane Jones as Ben — Night of the Living Dead film still, not a portrait" => {
                "duane-jones.jpg"
            }
            "Buster Keaton as Johnnie Gray — The General film still, not a portrait" => {
                "buster-keaton.jpg"
            }
            "TMDB approved primary short blue logo" => "tmdb.png",
            label => panic!("noncanonical artwork: {label}"),
        };
        for variant in ["native", "original"] {
            let (url, file) = if variant == "native" {
                (image.url.clone(), filename)
            } else {
                (
                    format!("{}/original", image.url.rsplit_once('/').unwrap().0),
                    if filename == "tmdb.png" {
                        "tmdb.svg"
                    } else {
                        filename
                    },
                )
            };
            let trusted = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../fixtures/catalog/artwork")
                    .join(file),
            )
            .unwrap();
            if file == "tmdb.png" {
                assert_eq!(trusted.len(), 10264);
            }
            assert_eq!(client.clone().load_artwork(url).await.unwrap(), trusted);
        }
    }
    assert_eq!(ids.len(), 21);
    client.invalidate_reads();
    assert_eq!(
        client
            .clone()
            .get_item("nosferatu".into(), scope())
            .await
            .unwrap()
            .year,
        1922
    );
    client.cancel_all();
    tokio::task::spawn_blocking(move || {
        let process_budget = Duration::from_secs(120)
            .saturating_sub(started.elapsed())
            .min(Duration::from_secs(60));
        let mut command = std::process::Command::new("python3");
        command
            .arg("scripts/backend-contract.py")
            .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
        support::process(command, process_budget, 1024 * 1024)
    })
    .await
    .unwrap()
    .unwrap();
}
