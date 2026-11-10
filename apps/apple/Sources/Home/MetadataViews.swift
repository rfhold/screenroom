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
                guard let data = try await catalog.client?.loadArtwork(url: url) else { throw CocoaError(.fileReadUnknown) }
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
                    .accessibilityLabel("\(provider.provider): \(provider.status)\(record.rating?.provider.lowercased() == provider.provider.lowercased() ? "; user rating \(score)" : "")")
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
                Text(([ratingDetail, "Metadata snapshot · \(record.retrievedAt)", "Provider date: \(record.date)"] + record.providers.map { "\($0.provider) · \($0.status)" }).joined(separator: " · "))
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
            } else { Text(provider.status).foregroundStyle(.secondary) }
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

struct CreditsView: View {
    @Environment(CatalogStore.self) private var catalog
    @State private var expanded = false
    @State private var retry = 0
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Button("Data & artwork credits") { expanded.toggle() }.buttonStyle(.borderless).frame(minHeight: 44).accessibilityIdentifier("credits.toggle")
                .task(id: "\(expanded)|\(retry)|\(catalog.artworkRevision)") { if expanded { await catalog.loadProvenance() } }
            if expanded {
                Group {
                    if catalog.provenanceLoading { ProgressView("Loading provenance…") }
                    else if !catalog.provenanceError.isEmpty {
                        Text(catalog.provenanceError).accessibilityIdentifier("credits.error")
                        Button("Retry credits") { retry += 1 }.accessibilityIdentifier("credits.retry")
                    }
                }
                if let info = catalog.catalogInfo, !catalog.provenanceLoading && catalog.provenanceError.isEmpty {
                ForEach(info.branding.filter { $0.kind == "logo" }, id: \.id) { art in
                    CoreImage(url: art.url).scaledToFit().frame(width: 88, height: 40).accessibilityLabel(art.label).accessibilityIdentifier("credits.logo.\(art.id)")
                }
                ForEach(Array(info.notices.enumerated()), id: \.element.id) { index, notice in
                    VStack(alignment: .leading, spacing: 4) {
                        if !notice.section.isEmpty && (index == 0 || info.notices[index - 1].section != notice.section) { Text(notice.section).font(.headline) }
                        if !notice.title.isEmpty { Text(notice.title).font(.headline) }
                        Text(notice.text)
                        if let link = externalURL(notice.linkUrl) { Link(notice.linkLabel, destination: link) }
                    }.accessibilityIdentifier("credits.notice.\(notice.id)")
                }
                if !info.retrievedAt.isEmpty { Text("Snapshot · \(info.retrievedAt)") }
                ForEach(catalog.provenanceArtwork, id: \.id) { art in
                    VStack(alignment: .leading, spacing: 4) {
                        Text(art.label).bold()
                        Text(art.creator)
                        if let license = externalURL(art.licenseUrl) { Link(art.license, destination: license) }
                        else { Text(art.license) }
                        if let source = externalURL(art.source) { Link("Source & provenance", destination: source) }
                    }.accessibilityIdentifier("credits.artwork.\(art.id)")
                }
                if !catalog.provenanceItems.isEmpty { Text("Film & episode rights").font(.headline) }
                ForEach(catalog.provenanceItems) { item in
                    Text(item.title).bold()
                    Text(item.rights)
                    if let attribution = item.attribution { Text(attribution) }
                    if let source = externalURL(item.rightsSource) { Link("\(item.title) rights source", destination: source) }
                    if let value = item.licenseURL, let license = externalURL(value) { Link("License", destination: license) }
                    ForEach(Array(item.value.provenanceNotes.enumerated()), id: \.offset) { _, note in Text(note) }
                    if let rights = item.value.metadataRights {
                        Text("Metadata: \(rights.label) · \(rights.note)")
                        if !rights.attribution.isEmpty { Text(rights.attribution) }
                        if let source = externalURL(rights.source) { Link("Metadata rights source", destination: source) }
                        if let license = externalURL(rights.licenseUrl) { Link("Metadata license", destination: license) }
                    }
                }
                }
            }
        }.font(.caption).foregroundStyle(.secondary)
    }
}
