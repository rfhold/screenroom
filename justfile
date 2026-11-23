default:
    @just --list

build target:
    bash packaging/build.sh build {{ quote(target) }}

check target:
    bash packaging/build.sh check {{ quote(target) }}

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
