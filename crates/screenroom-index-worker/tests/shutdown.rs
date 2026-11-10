#[test]
fn early_post_telemetry_error_exits_with_failure() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_screenroom-index-worker"))
        .arg("invalid-command")
        .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
        .env_remove("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("index worker failed"));
}

#[test]
fn every_command_rejects_invalid_migration_configuration() {
    for args in [
        vec!["serve"],
        vec!["drain"],
        vec!["resume", "00000000-0000-0000-0000-000000000000"],
        vec!["activate-model"],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_screenroom-index-worker"))
            .args(args)
            .env("DATABASE_URL", "postgres://runtime@127.0.0.1:1/screenroom")
            .env(
                "SCREENROOM_MIGRATION_DATABASE_URL",
                "invalid-migration-secret",
            )
            .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
            .env_remove("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("index worker failed"));
        assert!(!stderr.contains("invalid-migration-secret"));
    }
}
