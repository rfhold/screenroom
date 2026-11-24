default:
    @just --list

build target:
    bash packaging/build.sh build {{ quote(target) }}

check target:
    bash packaging/build.sh check {{ quote(target) }}

[positional-arguments]
backend action *args:
    bash scripts/backend.sh "$@"

backend-test:
    bash scripts/backend-shell-tests.sh
    python3 scripts/backend-compose-tests.py
    python3 scripts/backend-packaging-tests.py

backend-contract:
    bash scripts/backend-contract.sh

backend-persistence owner_file:
    bash scripts/backend-persistence.sh {{ quote(owner_file) }}

proto-tools:
    cargo install --locked --root target/buf-tools protoc-gen-prost@0.5.0 protoc-gen-tonic@0.5.0

proto:
    buf lint
    PATH="$PWD/target/buf-tools/bin:$PATH" buf generate

package target version build:
    bash packaging/package.sh {{ quote(target) }} {{ quote(version) }} {{ quote(build) }}

publish release_dir mode="":
    bash packaging/publish.sh {{ quote(release_dir) }} {{ quote(mode) }}

publish-target release_dir target mode="":
    bash packaging/publish.sh {{ quote(release_dir) }} {{ quote(mode) }} {{ quote(target) }}
