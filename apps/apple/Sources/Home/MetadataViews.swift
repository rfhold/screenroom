import SwiftUI

struct CoreImage: View {
    let url: String
    @Environment(CatalogStore.self) private var catalog
    @State private var bytes: Data?
    @State private var failed = false
    var body: some View {
        Group {
            if let bytes {
                #if os(macOS)
                if let image = NSImage(data: bytes) { Image(nsImage: image).resizable() }
                #else
                if let image = UIImage(data: bytes) { Image(uiImage: image).resizable() }
                #endif
            } else if failed { Text("Artwork unavailable").font(.caption).accessibilityIdentifier("artwork.error") }
            else { ProgressView().accessibilityIdentifier("artwork.loading") }
        }
        .task(id: "\(url)|\(catalog.artworkRevision)") {
            bytes = nil; failed = false
            do {
                guard let client = catalog.client else { throw CocoaError(.fileReadUnknown) }
                let data = try await catalog.artworkLoader.load(key: "\(url)|\(catalog.artworkRevision)") { try await client.loadArtwork(url: url) }
                try Task.checkCancellation()
                #if os(macOS)
                let decodable = NSImage(data: data) != nil
                #else
                let decodable = UIImage(data: data) != nil
                #endif
                guard decodable else { throw CocoaError(.fileReadCorruptFile) }
                bytes = data
                catalog.loadedArtwork.insert(url)
            } catch { if !Task.isCancelled { failed = true; catalog.failedArtwork.insert(url) } }
        }
    }
}

struct ProviderRow: View {
    let record: MetadataRecord
    @State private var expanded = false
    @FocusState private var infoFocused: Bool
    private var score: String { record.score }
    private var ratingDetail: String { record.ratingDetail }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 12) {
                ForEach(Array(record.providers.enumerated()), id: \.offset) { _, provider in
                    Group {
                        if let url = externalURL(provider.url) {
                            Link(destination: url) { providerLabel(provider) }
                        } else { providerLabel(provider) }
                    }
                    #if os(tvOS)
                    .buttonStyle(.card)
                    #else
                    .buttonStyle(.plain)
                    #endif
                    .accessibilityLabel("\(provider.provider): \(providerStatus(provider.status))\(record.rating?.provider.lowercased() == provider.provider.lowercased() ? "; user rating \(score)" : "")")
                }
                if let rating = record.rating, !record.providers.contains(where: { $0.provider.lowercased() == rating.provider.lowercased() }) {
                    Text("\(rating.provider) · \(score)")
                }
                Button { expanded.toggle() } label: {
                    Image(systemName: "info.circle").font(.body).frame(width: ScreenroomTheme.controlSize, height: ScreenroomTheme.controlSize)
                        .background(infoFocused ? Color.white.opacity(0.12) : .clear, in: RoundedRectangle(cornerRadius: 8))
                        .contentShape(RoundedRectangle(cornerRadius: 8))
                }
                #if os(tvOS)
                .buttonStyle(.card)
                #else
                .buttonStyle(.plain)
                #endif
                .focused($infoFocused).accessibilityLabel("Rating and catalog details").accessibilityIdentifier("metadata.info")
            }
            if expanded {
                Text(([ratingDetail, "Metadata snapshot · \(record.retrievedAt)", "Provider date: \(record.date)"] + record.providers.map { "\($0.provider) · \(providerStatus($0.status))" }).joined(separator: " · "))
                    .font(.caption).foregroundStyle(.secondary).accessibilityIdentifier("metadata.disclosure")
            }
        }
    }

    private func providerLabel(_ provider: ProviderReference) -> some View {
        HStack(spacing: 5) {
            Text(provider.provider)
            if record.rating?.provider.lowercased() == provider.provider.lowercased() {
                Image(systemName: "star.fill").foregroundStyle(.yellow)
                Text(score)
            } else { Text(providerStatus(provider.status)).foregroundStyle(.secondary) }
        }
                        .font(.caption).padding(.horizontal, 8).frame(minHeight: ScreenroomTheme.controlSize)
                        .background(.white.opacity(0.05), in: RoundedRectangle(cornerRadius: 8))
                        .overlay(RoundedRectangle(cornerRadius: 8).stroke(.white.opacity(0.12)))
    }
}

struct PeopleView: View {
    let title: String
    let people: [Credit]
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title).font(.headline)
            if people.isEmpty { Text("No \(title.lowercased()) credits listed in this snapshot.").font(.caption).foregroundStyle(.secondary) }
            LazyVGrid(columns: [GridItem(.adaptive(minimum: ScreenroomTheme.personWidth - 12, maximum: ScreenroomTheme.personWidth + 24), spacing: 16, alignment: .top)], alignment: .leading, spacing: 24) {
                ForEach(Array(people.enumerated()), id: \.offset) { _, person in
                    VStack(alignment: .leading, spacing: 5) {
                        if let art = person.artwork {
                            CoreImage(url: art.url).scaledToFill().frame(width: ScreenroomTheme.personWidth - 12, height: ScreenroomTheme.personWidth - 12).clipped().clipShape(RoundedRectangle(cornerRadius: 8)).accessibilityLabel(art.label)
                        } else {
                            Text(person.name.split(separator: " ").prefix(2).compactMap(\.first).map(String.init).joined())
                                .frame(width: ScreenroomTheme.personWidth - 12, height: ScreenroomTheme.personWidth - 12).background(.white.opacity(0.05), in: RoundedRectangle(cornerRadius: 8)).accessibilityLabel("No portrait included for \(person.name)")
                        }
                        Text(person.name).font(.caption.weight(.semibold)).fixedSize(horizontal: false, vertical: true)
                        Text(person.role).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                        if person.artwork?.kind == "character still" { Text("Character still").font(.caption2).foregroundStyle(.secondary) }
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
    }
}

struct JellyfinProfileView: View {
    @Environment(CatalogStore.self) private var catalog
    @Bindable var profile: JellyfinProfileStore
    private enum Action { case load(String?), change(String?) }
    @State private var action = Action.load(nil)
    @State private var operationID = 0
    private func request(_ action: Action) {
        self.action = action
        operationID += 1
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Jellyfin profile").font(.headline)
            Text("Server-saved progress mapping for this local user selector. This is not sign-in.").font(.caption).foregroundStyle(.secondary)
            if profile.busy { ProgressView("Loading Jellyfin profile settings…") }
            if !profile.error.isEmpty {
                Text(profile.error)
                Button("Retry Jellyfin settings") { request(.load(nil)) }.disabled(profile.busy)
            }
            if !profile.status.isEmpty { Text(profile.status).foregroundStyle(.secondary) }
            if !profile.sources.isEmpty {
                Picker("Jellyfin source", selection: Binding(get: { profile.source }, set: { request(.load($0)) })) {
                    ForEach(profile.sources, id: \.id) { Text($0.displayName).tag($0.id) }
                }.disabled(profile.busy)
                Text("Current profile: \(profile.currentProfile)")
                Picker("Profile", selection: $profile.selected) {
                    if !profile.profiles.contains(where: { $0.id == profile.selected }) { Text("Choose a profile").tag(profile.selected) }
                    ForEach(profile.profiles, id: \.id) { Text($0.displayName).tag($0.id) }
                }.disabled(profile.busy || !profile.known || profile.profiles.isEmpty)
                if profile.known && profile.profiles.isEmpty { Text("No enabled profiles are available.") }
                Button("Save Jellyfin profile") { request(.change(profile.selected)) }
                    .disabled(profile.busy || !profile.known || !profile.profiles.contains { $0.id == profile.selected })
                Button("Unlink Jellyfin profile") { request(.change(nil)) }
                    .disabled(profile.busy || !profile.known || profile.link == nil)
            } else if !profile.busy && profile.error.isEmpty { Text("No Jellyfin sources are available.") }
        }
        .task(id: operationID) {
            switch action {
            case let .load(target): await profile.load(client: catalog.profileClient, target: target)
            case let .change(selected): await profile.change(client: catalog.profileClient, profile: selected)
            }
        }
        .onDisappear {
            profile.deactivate()
            action = .load(nil)
        }
    }
}
