import SwiftUI

#if os(tvOS)
private struct TVSidebarStyle: ButtonStyle {
    @Environment(\.isFocused) private var focused
    var selected = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(focused || selected ? .white : .white.opacity(0.72))
            .background(focused ? Color.white.opacity(0.20) : selected ? Color.white.opacity(0.08) : .clear, in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(focused ? .white.opacity(0.85) : .clear, lineWidth: 2))
    }
}
#endif

private struct CompactNavigationStyle: ButtonStyle {
    let focused: Bool
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.foregroundStyle(.white)
            .background(focused || configuration.isPressed ? Color.white.opacity(0.14) : .clear, in: RoundedRectangle(cornerRadius: 8))
    }
}

struct HomeView: View {
    @State private var catalog = CatalogStore()
    @State private var refresh = 0
    @State private var completedRefresh = 0
    @State private var navigationTask: Task<Void, Never>?
    @State private var navigationGeneration = 0
    @State private var expanded = false
    @State private var screen = "Home"
    @State private var library = "all"
    @State private var path: [CatalogItem] = []
    @State private var query = ""
    @State private var settled = ""
    @State private var options = SearchOptions()
    @State private var settings = false
    @State private var subtitle = "Off"
    #if os(tvOS)
    @State private var subtitleChoices = false
    #endif
    @State private var status = ""
    @State private var searchOrigin = "Home"
    @State private var searchOriginPath: [CatalogItem] = []
    @FocusState private var navigationFocus: String?
    @FocusState private var sidebarToggleFocused: Bool
    private var persistedSubtitle: String {
        catalog.preferences?.subtitlePreference == "library_tracks" ? "Library tracks" : catalog.preferences?.subtitlePreference == "transcription" ? "Transcription" : "Off"
    }

    var body: some View {
        GeometryReader { geometry in
            let compact = geometry.size.width < 600
            Group {
                if compact {
                    VStack(spacing: 0) { sidebar(compact: true); content(compact: true, width: geometry.size.width - 32) }
                } else {
                    HStack(alignment: .top, spacing: 0) {
                        sidebar(compact: false)
                        content(compact: false, width: geometry.size.width - (expanded ? 264 : ScreenroomTheme.controlSize + 24) - ScreenroomTheme.inset * 2)
                    }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(ScreenroomTheme.content)
        }
        .preferredColorScheme(.dark)
        .tint(.white)
        .environment(catalog)
        .task(id: "\(screen)|\(library)|\(path.last?.id ?? "")|\(refresh)|\(query)|\(options)") {
            let requested = query
            let requestedRefresh = refresh
            let refreshing = requestedRefresh > completedRefresh
            if refreshing { status = "" }
            await catalog.load(screen: screen, library: library, itemID: path.last?.id, forceBootstrap: refreshing)
            if screen == "Search" && path.isEmpty && catalog.error.isEmpty && !Task.isCancelled {
                await catalog.search(query: requested, library: library, options: options)
                if !Task.isCancelled { settled = requested }
            }
            if refreshing && !Task.isCancelled {
                completedRefresh = requestedRefresh
                status = catalog.error.isEmpty ? "Catalog refreshed from core." : "Refresh failed."
            }
        }
        .onChange(of: catalog.capabilities) { _, capabilities in
            if let capabilities { options.reconcile(capabilities, preferences: catalog.preferences) }
        }
        .onChange(of: catalog.preferences) { _, prefs in
            if let types = prefs?.searchTypes {
                options.types = Set([(types.movie, "Movie"), (types.series, "Series"), (types.episode, "Episode")].filter { $0.0 }.map { $0.1 })
                if let caps = catalog.capabilities { options.reconcile(caps, preferences: prefs) }
            }
            subtitle = prefs?.subtitlePreference == "library_tracks" ? "Library tracks" : prefs?.subtitlePreference == "transcription" ? "Transcription" : "Off"
        }
        .task(id: "\(options.types.sorted())|\(subtitle)|\(catalog.preferences != nil)") {
            if catalog.preferences != nil {
                await catalog.save(types: options.coreTypes, subtitle: subtitle == "Library tracks" ? "library_tracks" : subtitle == "Transcription" ? "transcription" : "off")
            }
        }
        .onDisappear { navigationGeneration += 1; navigationTask?.cancel() }
        #if os(tvOS)
        .onExitCommand {
            if !path.isEmpty { setPath(Array(path.dropLast())) }
            else if settings && screen == "Search" { settings = false }
            else if expanded { expanded = false }
            else if screen != "Home" { go("Home") }
        }
        #endif
    }

    private func setPath(_ value: [CatalogItem]) {
        navigationGeneration += 1
        navigationTask?.cancel()
        path = value
    }

    private func reload() {
        navigationGeneration += 1
        navigationTask?.cancel()
        catalog.invalidate()
        refresh += 1
    }

    private func go(_ destination: String) {
        navigationGeneration += 1
        navigationTask?.cancel()
        if destination == "Home" { library = "all" }
        screen = destination
        path = []
        status = ""
    }

    private func open(_ item: CatalogItem) {
        navigationGeneration += 1
        let current = navigationGeneration
        navigationTask?.cancel()
        navigationTask = Task {
            guard let client = catalog.client else { return }
            do {
                var value = try await client.getItem(id: item.id)
                try Task.checkCancellation()
                var ancestors = [CatalogItem(value: value)]
                var seen: Set<String> = [value.id]
                while let parent = value.parentId {
                    try Task.checkCancellation()
                    guard seen.insert(parent).inserted else { throw CancellationError() }
                    value = try await client.getItem(id: parent)
                    try Task.checkCancellation()
                    ancestors.insert(CatalogItem(value: value), at: 0)
                }
                try Task.checkCancellation()
                guard current == navigationGeneration else { return }
                setPath(ancestors)
            } catch {
                if !Task.isCancelled && current == navigationGeneration { catalog.error = "Unable to open title: \(error.localizedDescription)" }
            }
        }
    }

    private func startSearch() {
        if screen != "Search" {
            searchOrigin = screen
            searchOriginPath = path
        }
        query = ""
        settled = ""
        options.methods = Set(catalog.capabilities?.methods.filter { $0 != "title_only" } ?? [])
        options.titleOnly = false
        options.source = catalog.capabilities?.sources.first ?? ""
        settings = false
        go("Search")
    }

    private func menuButton(_ title: String, _ icon: String, libraryID: String? = nil, action: @escaping () -> Void) -> some View {
        let selected = (screen == "Home" && title == "Home") || (screen == "Search" && title == "Search titles") || (screen == "Library" && libraryID == library)
        #if os(tvOS)
        return tvSidebarRow(title, icon, selected: selected, action: action)
            .accessibilityValue(selected ? "Selected" : "Not selected")
            .accessibilityIdentifier("menu.\(title)")
        #else
        return Button(action: action) {
            HStack(spacing: 12) {
                Image(systemName: icon).frame(width: 24)
                if expanded { Text(title).font(.body); Spacer(minLength: 0) }
            }
            .frame(minWidth: ScreenroomTheme.controlSize, minHeight: ScreenroomTheme.controlSize)
            .foregroundStyle(selected ? .white : .white.opacity(0.65))
            .background(selected ? .white.opacity(0.05) : .clear, in: RoundedRectangle(cornerRadius: 8))
            .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .accessibilityLabel(title)
        .accessibilityIdentifier("menu.\(title)")
        #endif
    }

    private var navigationButtons: some View {
        Group {
            menuButton("Home", "house") { go("Home") }
            menuButton("Search titles", "magnifyingglass") { startSearch() }
            if expanded { Text("Libraries").font(.caption).foregroundStyle(.secondary).padding(.top, 12) }
            ForEach(catalog.libraries, id: \.id) { value in
                menuButton(value.title, value.id == "tv" ? "tv" : "rectangle.stack", libraryID: value.id) { library = value.id; go("Library") }
            }
        }
    }

    private func sidebar(compact: Bool) -> some View {
        #if os(tvOS)
        tvSidebar
        #else
        VStack(alignment: .leading, spacing: 8) {
            if compact && !expanded {
                HStack(spacing: 0) { sidebarToggle; navigationButtons }
            } else {
                sidebarToggle
                ScrollView {
                    VStack(alignment: .leading, spacing: 8) {
                        navigationButtons
                        if expanded {
                            Divider().padding(.vertical, 12)
                            Text("Subtitles").foregroundStyle(.secondary)
                            ForEach(["Off", "Library tracks", "Transcription"], id: \.self) { choice in
                                Button { subtitle = choice } label: {
                                    Label(choice, systemImage: subtitle == choice ? "checkmark.circle.fill" : "circle")
                                        .frame(maxWidth: .infinity, minHeight: ScreenroomTheme.controlSize, alignment: .leading)
                                 }.buttonStyle(.borderless)
                                     .accessibilityIdentifier("subtitle.\(choice)")
                                     .accessibilityValue(subtitle == choice ? persistedSubtitle == choice ? "Selected" : "Saving" : "Not selected")
                            }
                             Button("Refresh") { reload() }.frame(minHeight: ScreenroomTheme.controlSize)
                        }
                    }
                }
            }
        }
        .padding(compact ? 8 : 12)
        .frame(width: compact ? nil : (expanded ? 264 : ScreenroomTheme.controlSize + 24), alignment: .leading)
        .frame(height: compact && expanded ? 360 : nil, alignment: .topLeading)
        .frame(maxWidth: compact ? .infinity : nil, maxHeight: compact ? nil : .infinity, alignment: .topLeading)
        .background(ScreenroomTheme.sidebar)
        #if os(tvOS)
        .focusSection()
        #endif
        #endif
    }

    #if os(tvOS)
    private func tvSidebarRow(_ title: String, _ icon: String, selected: Bool = false, value: String? = nil, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Image(systemName: icon).font(.system(size: 24, weight: .regular))
                    .frame(width: 28, height: 28)
                if expanded {
                    VStack(alignment: .leading, spacing: 1) {
                        Text(title == "Search titles" ? "Search" : title).font(.system(size: 23, weight: .medium)).lineLimit(1)
                        if let value { Text(value).font(.system(size: 17)).foregroundStyle(.secondary).lineLimit(1) }
                    }
                    Spacer(minLength: 0)
                }
            }
            .padding(.horizontal, 12)
            .frame(maxWidth: .infinity, minHeight: 56, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(TVSidebarStyle(selected: selected))
        .accessibilityLabel(title)
    }

    private var tvSidebar: some View {
        VStack(alignment: .leading, spacing: 8) {
            sidebarToggle
            ScrollView {
                VStack(alignment: .leading, spacing: 6) {
                    menuButton("Home", "house") { go("Home") }
                    menuButton("Search titles", "magnifyingglass") { startSearch() }
                    if expanded {
                        Text("Libraries").font(.system(size: 18, weight: .medium)).foregroundStyle(.secondary)
                            .padding(.horizontal, 12).padding(.top, 14).padding(.bottom, 4)
                    }
                    ForEach(catalog.libraries, id: \.id) { value in
                        menuButton(value.title, value.id == "tv" ? "tv" : value.id == "movies" ? "film" : "rectangle.stack", libraryID: value.id) { library = value.id; go("Library") }
                    }
                }
            }
            if expanded {
                VStack(alignment: .leading, spacing: 6) {
                    Divider().padding(.bottom, 6)
                    tvSidebarRow("Subtitles", subtitleChoices ? "chevron.up" : "captions.bubble", value: subtitle) { subtitleChoices.toggle() }
                        .accessibilityIdentifier("sidebar.subtitles")
                        .accessibilityValue("\(persistedSubtitle); \(subtitleChoices ? "Expanded" : "Collapsed")")
                    if subtitleChoices {
                        ForEach(["Off", "Library tracks", "Transcription"], id: \.self) { choice in
                            tvSidebarRow(choice, subtitle == choice ? "checkmark" : "circle", selected: subtitle == choice) {
                                subtitle = choice
                                subtitleChoices = false
                            }.accessibilityIdentifier("subtitle.\(choice)")
                        }
                    }
                    tvSidebarRow("Refresh", "arrow.clockwise") { reload() }
                }
            }
        }
        .padding(12)
        .frame(width: expanded ? 264 : ScreenroomTheme.controlSize + 24)
        .frame(maxHeight: .infinity, alignment: .topLeading)
        .background(ScreenroomTheme.sidebar)
        .focusSection()
    }
    #endif

    private var sidebarToggle: some View {
        Button { expanded.toggle() } label: {
            Image(systemName: "sidebar.left")
                .font(.system(size: 20, weight: .medium)).symbolRenderingMode(.monochrome)
                .frame(width: ScreenroomTheme.controlSize, height: ScreenroomTheme.controlSize)
        }
        #if os(tvOS)
        .buttonStyle(CompactNavigationStyle(focused: sidebarToggleFocused))
        .focused($sidebarToggleFocused)
        #else
        .buttonStyle(CompactNavigationStyle(focused: navigationFocus == "sidebar.toggle"))
        .focused($navigationFocus, equals: "sidebar.toggle")
        #endif
        .accessibilityLabel(expanded ? "Collapse sidebar" : "Expand sidebar")
        .accessibilityIdentifier("sidebar.toggle")
    }

    private func content(compact: Bool, width: CGFloat) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                if !status.isEmpty { Text(status).foregroundStyle(.secondary).accessibilityIdentifier("refresh.status") }
                if !catalog.error.isEmpty {
                    Text(catalog.error).accessibilityIdentifier("core.error")
                    Button("Retry") { reload() }.accessibilityIdentifier("core.retry")
                }
                if catalog.loading { ProgressView("Loading catalog…").accessibilityIdentifier("core.loading") }
                if !path.isEmpty {
                    breadcrumbs
                    if let item = path.last {
                        detail(catalog.detail?.id == item.id ? catalog.detail! : item, width: width, compact: compact)
                    }
                } else if screen == "Home" {
                    heading("Home")
                    ForEach(Array(catalog.rows.enumerated()), id: \.offset) { index, row in
                        VStack(alignment: .leading, spacing: 14) {
                            Text(row.title).font(.title3.weight(.semibold)).accessibilityIdentifier("home.row.\(index)")
                            ScrollView(.horizontal) {
                                HStack(alignment: .top, spacing: 18) {
                                    ForEach(row.items.map(CatalogItem.init)) { item in card(item, width: compact ? 140 : ScreenroomTheme.posterWidth) }
                                }.padding(.vertical, 8)
                            }
                        }
                    }
                } else if screen == "Library" {
                    heading(catalog.libraryTitle(library))
                    grid(catalog.items, compact: compact)
                } else if screen == "Search" {
                    search(compact: compact)
                }
                 CreditsView().frame(maxWidth: ScreenroomTheme.detailMaxWidth, alignment: .leading)
             }
             .padding(compact ? 16 : ScreenroomTheme.inset)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        #if os(tvOS)
        .focusSection()
        #endif
    }

    private func heading(_ title: String) -> some View {
        Text(title).font(.largeTitle.weight(.semibold)).accessibilityAddTraits(.isHeader).accessibilityIdentifier("page.title")
    }

    private var breadcrumbs: some View {
        ScrollView(.horizontal) {
            HStack {
                Button { setPath(Array(path.dropLast())) } label: {
                    HStack(spacing: 8) {
                        Image(systemName: "chevron.left").font(.system(size: 18, weight: .medium)).symbolRenderingMode(.monochrome)
                        Text("Back")
                    }.padding(.horizontal, 8).frame(minHeight: ScreenroomTheme.controlSize)
                }
                .buttonStyle(CompactNavigationStyle(focused: navigationFocus == "navigation.back"))
                .focused($navigationFocus, equals: "navigation.back")
                #if os(tvOS)
                .onMoveCommand { direction in
                    if direction == .left { sidebarToggleFocused = true }
                }
                #endif
                .accessibilityIdentifier("navigation.back")
                Button(screen == "Library" ? catalog.libraryTitle(library) : screen) { setPath([]) }
                    .buttonStyle(CompactNavigationStyle(focused: navigationFocus == "breadcrumb.root"))
                    .focused($navigationFocus, equals: "breadcrumb.root")
                    .accessibilityIdentifier("breadcrumb.root")
                ForEach(Array(path.enumerated()), id: \.element.id) { index, item in
                    Image(systemName: "chevron.right").foregroundStyle(.secondary)
                    Button(item.title) { setPath(Array(path.prefix(index + 1))) }
                        .buttonStyle(CompactNavigationStyle(focused: navigationFocus == item.id))
                        .focused($navigationFocus, equals: item.id)
                        .accessibilityIdentifier("breadcrumb.\(item.id)")
                }
            }.frame(minHeight: ScreenroomTheme.controlSize).padding(.vertical, 8)
            #if !os(tvOS)
            .padding(.horizontal, 8)
            #endif
        }
    }

    private func card(_ item: CatalogItem, width: CGFloat) -> some View {
        Button { open(item) } label: {
            VStack(alignment: .leading, spacing: 8) {
                PosterView(item: item).frame(width: width, height: width * 1.5)
                Text(item.title).font(.body.weight(.medium)).lineLimit(2)
                Text("\(item.kind) · \(String(item.year))").font(.caption).foregroundStyle(.secondary)
            }.frame(width: width, alignment: .leading).contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .accessibilityLabel("\(item.title), \(item.kind), \(item.meta)")
        .accessibilityValue(item.artwork == nil ? "Designed placeholder; poster not included" : catalog.loadedArtwork.contains(item.artwork!.url) ? "Licensed core artwork ready" : "Artwork loading")
        .accessibilityIdentifier("card.\(item.id)")
    }

    private func grid(_ items: [CatalogItem], compact: Bool) -> some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: compact ? 140 : ScreenroomTheme.posterWidth), spacing: 20)], alignment: .leading, spacing: 24) {
            ForEach(items) { item in card(item, width: compact ? 140 : ScreenroomTheme.posterWidth) }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        #if os(tvOS)
        .focusSection()
        #endif
    }

    private func detail(_ item: CatalogItem, width: CGFloat, compact: Bool) -> some View {
        VStack(alignment: .leading, spacing: 28) {
            if width >= 640 {
                HStack(alignment: .top, spacing: 30) {
                    PosterView(item: item).frame(width: ScreenroomTheme.detailPosterWidth, height: ScreenroomTheme.detailPosterWidth * 1.5)
                    detailSummary(item).frame(maxWidth: .infinity, alignment: .leading)
                }
                #if os(tvOS)
                .focusSection()
                #endif
            } else {
                VStack(alignment: .leading, spacing: 24) {
                    PosterView(item: item).frame(width: 160, height: 240)
                    detailSummary(item)
                }
            }
            if catalog.detail?.id == item.id {
                if item.kind == "Series" || item.kind == "Season" { grid(catalog.children, compact: compact) }
                PeopleView(title: "Cast", people: catalog.credits.filter { $0.department.lowercased() == "cast" })
                PeopleView(title: "Crew", people: catalog.credits.filter { $0.department.lowercased() != "cast" })
            }
            VStack(alignment: .leading, spacing: 12) {
                Text("Title rights").font(.headline)
                Text(item.rights).font(.caption).foregroundStyle(.secondary)
                if let attribution = item.attribution { Text(attribution).font(.caption) }
                if let source = externalURL(item.rightsSource) { Link("Rights source", destination: source) }
                if let value = item.licenseURL, let license = externalURL(value) { Link("License", destination: license) }
                ForEach(Array(item.value.provenanceNotes.enumerated()), id: \.offset) { _, note in Text(note).font(.caption) }
                if let rights = item.value.metadataRights {
                    Text("Metadata: \(rights.label) · \(rights.note)").font(.caption)
                    if !rights.attribution.isEmpty { Text(rights.attribution).font(.caption) }
                    if let source = externalURL(rights.source) { Link("Metadata rights source", destination: source) }
                    if let license = externalURL(rights.licenseUrl) { Link("Metadata license", destination: license) }
                }
            }
        }.frame(maxWidth: ScreenroomTheme.detailMaxWidth, alignment: .leading)
    }

    private func detailSummary(_ item: CatalogItem) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("\(item.kind) · \(String(item.year))").foregroundStyle(.secondary)
            heading(item.title)
            Text(item.meta).foregroundStyle(.secondary)
            if let record = item.record {
                if item.kind == "Episode" { Text("Aired \(record.date)").font(.caption).foregroundStyle(.secondary) }
                ProviderRow(record: record)
            }
            Text(item.overview).fixedSize(horizontal: false, vertical: true)
        }
    }

    private func selection(_ title: String, selected: Bool, disabled: Bool = false, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Label(title, systemImage: selected ? "checkmark.square.fill" : "square").frame(minHeight: ScreenroomTheme.controlSize, alignment: .leading)
        }.buttonStyle(.borderless).disabled(disabled).accessibilityValue(selected ? "Selected" : "Not selected")
    }

    private func search(compact: Bool) -> some View {
        VStack(alignment: .leading, spacing: 20) {
            Button("Back to \(searchOrigin == "Library" ? catalog.libraryTitle(library) : searchOrigin)", systemImage: "chevron.left") {
                screen = searchOrigin
                setPath(searchOriginPath)
            }.frame(minHeight: ScreenroomTheme.controlSize).accessibilityIdentifier("search.back")
            heading("Search")
            TextField("Search \(catalog.libraryTitle(library))", text: $query).accessibilityIdentifier("search.query")
                .padding(14).background(ScreenroomTheme.sidebar, in: RoundedRectangle(cornerRadius: 8))
                .onChange(of: query) { _, value in
                    if let limit = catalog.capabilities?.maximumQueryLength, value.count > limit { query = String(value.prefix(Int(limit))) }
                    if value.isEmpty { settled = "" }
                }
            if !query.isEmpty { Button("Clear query") { query = ""; settled = "" } }
             Button("Settings", systemImage: "slider.horizontal.3") { settings.toggle() }
                .frame(minHeight: ScreenroomTheme.controlSize).accessibilityIdentifier("search.options")
            if settings {
                 VStack(alignment: .leading, spacing: 12) {
                     Text("Catalog").font(.headline)
                     HStack {
                          ForEach(catalog.capabilities?.sources ?? [], id: \.self) { source in
                              selection(choiceLabel(source), selected: options.source == source) { options.source = source }
                                  .accessibilityIdentifier("search.source.\(choiceLabel(source))")
                         }
                     }
                     Text("Catalog membership, not playback availability.").font(.caption).foregroundStyle(.secondary)
                     Text("Media types").font(.headline)
                     ViewThatFits(in: .horizontal) {
                         HStack { typeChoices }
                         VStack(alignment: .leading) { typeChoices }
                     }
                    Text("Search methods").font(.headline)
                    Text("Keep at least one method selected.").font(.caption).foregroundStyle(.secondary)
                     if catalog.capabilities?.methods.contains("title_only") == true { selection("Title only", selected: options.titleOnly) {
                        options.titleOnly.toggle()
                        if options.titleOnly {
                             options.methods.remove("semantic")
                             if options.methods.isEmpty, let method = catalog.capabilities?.methods.first(where: { $0 != "semantic" && $0 != "title_only" }) { options.methods.insert(method) }
                        }
                     } }
                     ForEach(catalog.capabilities?.methods.filter { $0 != "title_only" } ?? [], id: \.self) { method in
                         selection(choiceLabel(method), selected: options.methods.contains(method), disabled: (method == "semantic" && options.titleOnly) || (options.methods.contains(method) && options.methods.count == 1)) {
                            if options.methods.contains(method) { options.methods.remove(method) } else { options.methods.insert(method) }
                         }.accessibilityIdentifier("search.method.\(choiceLabel(method))")
                    }
                     if let description = catalog.capabilities?.semanticDescription { Text(description).font(.caption).foregroundStyle(.secondary) }
                }.padding(20).background(ScreenroomTheme.sidebar, in: RoundedRectangle(cornerRadius: 12))
            }
            let results = catalog.results
              Text(query.trimmingCharacters(in: .whitespaces).isEmpty ? "Enter a title to search \(catalog.libraryTitle(library))." : query != settled ? "Searching titles…" : results.isEmpty ? "No matches in \(catalog.libraryTitle(library)). Try another search." : "\(results.count) \(results.count == 1 ? "result" : "results") in \(catalog.libraryTitle(library))")
                .foregroundStyle(.secondary).accessibilityIdentifier("search.status")
            if !query.isEmpty && query == settled { grid(results, compact: compact) }
        }
    }

    private var typeChoices: some View {
        ForEach(catalog.capabilities?.types ?? [], id: \.self) { kind in
            selection(choiceLabel(kind), selected: options.types.contains(kind), disabled: options.types.contains(kind) && options.types.count == 1) {
                 if options.types.contains(kind) { options.types.remove(kind) } else { options.types.insert(kind) }
            }.accessibilityIdentifier("search.type.\(kind)")
        }
    }
}

struct PosterView: View {
    let item: CatalogItem

    var body: some View {
        GeometryReader { geometry in
            let width = geometry.size.width
             ZStack(alignment: .topLeading) {
                if item.artwork == nil {
                LinearGradient(colors: [Color(hex: item.ink), Color(hex: item.light).opacity(0.65)], startPoint: .topLeading, endPoint: .bottomTrailing)
                Circle().stroke(Color(hex: item.light).opacity(0.6), lineWidth: 1)
                    .frame(width: width * 0.85, height: width * 0.85).offset(x: width * 0.28, y: geometry.size.height * 0.14)
                RoundedRectangle(cornerRadius: width * 0.4)
                    .fill(LinearGradient(colors: [.black.opacity(0.15), Color(hex: item.light)], startPoint: .leading, endPoint: .trailing))
                    .frame(width: width * 1.7, height: geometry.size.height).rotationEffect(.degrees(-32)).opacity(0.45)
                    .offset(x: -width * 0.2, y: geometry.size.height * 0.48)
                Text(item.kind == "Episode" ? String(item.meta.split(separator: "·")[0]).trimmingCharacters(in: .whitespaces) : item.title)
                    .font(.system(size: width * 0.155, design: .serif)).lineLimit(3).minimumScaleFactor(0.6).foregroundStyle(Color(hex: "fff5e5"))
                    .frame(width: width * 0.78, alignment: .leading).offset(x: width * 0.12, y: geometry.size.height * 0.42)
                  Text("POSTER NOT INCLUDED")
                    .font(.system(size: max(6, width * 0.038))).tracking(2).foregroundStyle(.white.opacity(0.6))
                     .offset(x: width * 0.12, y: geometry.size.height - 24)
                }
                 if let art = item.artwork {
                      CoreImage(url: art.url).scaledToFill().frame(width: width, height: geometry.size.height).clipped()
                 }
                 if let progress = item.progress {
                    VStack { Spacer(); ZStack(alignment: .leading) {
                        Rectangle().fill(.white.opacity(0.2))
                        Rectangle().fill(.white.opacity(0.9)).frame(width: width * progress / 100)
                    }.frame(height: 4) }
                }
            }
            .frame(width: width, height: geometry.size.height, alignment: .topLeading)
            .clipShape(RoundedRectangle(cornerRadius: 10))
        }.accessibilityHidden(true)
    }
}
