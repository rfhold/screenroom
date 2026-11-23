/// Trusted-local user UUID selector; this metadata does not authenticate callers.
pub const LOCAL_USER_ID_METADATA_KEY: &str = "x-screenroom-user-id";

pub mod catalog {
    pub mod v1 {
        include!("generated/screenroom/catalog/v1/screenroom.catalog.v1.rs");
    }
}
pub mod common {
    pub mod v1 {
        include!("generated/screenroom/common/v1/screenroom.common.v1.rs");
    }
}
pub mod playback_gateway {
    pub mod v1 {
        include!("generated/screenroom/playback_gateway/v1/screenroom.playback_gateway.v1.rs");
    }
}
pub mod operator {
    pub mod v1 {
        include!("generated/screenroom/operator/v1/screenroom.operator.v1.rs");
    }
}
pub mod user {
    pub mod v1 {
        include!("generated/screenroom/user/v1/screenroom.user.v1.rs");
    }
    pub mod internal {
        pub mod v1 {
            include!("generated/screenroom/user/internal/v1/screenroom.user.internal.v1.rs");
        }
    }
}
