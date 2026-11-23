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
    @State private var catalog: CatalogStore
    @State private var refresh = 0
    @State private var completedRefresh = 0
    @State private var navigationTask: Task<Void, Never>?
    @State private var failedOpen: CatalogItem?
    @State private var navigationGeneration = 0
    @State private var expanded = false
    @State private var screen = "Home"
    @State private var library = "all"
    @State private var path: [CatalogItem] = []
    @State private var query = ""
    @State private var settled = ""
    @State private var options = SearchOptions()
    @State private var settings = false
    #if os(tvOS)
    #endif
    @State private var status = ""
    @State private var searchOrigin = "Home"
    @State private var searchOriginPath: [CatalogItem] = []
    @State private var playback: NativePlayback?
    @State private var playbackTitle = ""
    @FocusState private var navigationFocus: String?
    @FocusState private var sidebarToggleFocused: Bool

    init() {
        let store = CatalogStore()
        _catalog = State(initialValue: store)
        let saved = store.preferences ?? defaultSettings()
        let types = saved.types ?? SearchTypes(movie: true, series: true, episode: false)
        let methods = saved.methods ?? SearchMethods(titleOnly: false, text: true, fuzzy: true, semantic: true)
        _options = State(initialValue: SearchOptions(source: saved.source, types: Set([(types.movie, "Movie"), (types.series, "Series"), (types.episode, "Episode")].filter { $0.0 }.map { $0.1 }), methods: Set([(methods.text, "text"), (methods.fuzzy, "fuzzy"), (methods.semantic, "semantic")].filter { $0.0 }.map { $0.1 }), titleOnly: methods.titleOnly))
        let savedLibrary = saved.scope?.libraryId ?? ""
        _library = State(initialValue: savedLibrary.isEmpty ? "all" : savedLibrary)
        _screen = State(initialValue: savedLibrary.isEmpty ? "Home" : "Library")
    }

    private func saveSettings() {
        catalog.save(options: options, library: library)
    }

    var body: some View {
        GeometryReader { geometry in
            let compact = geometry.size.width < 600
            Group {
                if let playback {
                    NativePlaybackView(playback: playback, title: playbackTitle) {
                        self.playback = nil
                        catalog.invalidate()
                        refresh += 1
                    }
                } else if compact {
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
        .task(id: "\(catalog.scope.sourceId)|\(screen)|\(library)|\(path.last?.id ?? "")|\(refresh)|\(query)|\(options)|\(catalog.jellyfinProfile.revision)") {
            let requested = query
            let requestedRefresh = refresh
            let refreshing = requestedRefresh > completedRefresh
            if refreshing { status = "" }
            let accepted = await catalog.visit(viewKey, options: options, forceBootstrap: refreshing)
            if accepted && screen == "Search" && path.isEmpty { settled = requested }
            if refreshing && !Task.isCancelled {
                completedRefresh = requestedRefresh
                status = catalog.error.isEmpty ? "Catalog refreshed from core." : "Refresh failed."
            }
        }
        .onChange(of: catalog.capabilities) { _, capabilities in
            if let capabilities { options.reconcile(capabilities, preferences: catalog.preferences) }
        }
        .onChange(of: library) { _, _ in saveSettings() }
        .onChange(of: catalog.jellyfinProfile.revision) { _, _ in
            if let playback { Task { await playback.close(reason: "profile_changed") } }
        }
        .onDisappear {
            navigationGeneration += 1
            navigationTask?.cancel()
            if let playback {
                #if os(iOS)
                // The fullscreen presenter stops a true unmount without treating a cover as exit.
                if !playback.presentation.fullscreen { Task { await playback.close(reason: "unmount") } }
                #else
                Task { await playback.close(reason: "unmount") }
                #endif
            }
        }
        #if os(tvOS)
        .onExitCommand {
            guard playback == nil else { return }
            if !path.isEmpty { setPath(Array(path.dropLast())) }
            else if settings && screen == "Search" { settings = false }
            else if expanded { expanded = false }
            else if screen != "Home" { go("Home") }
        }
        #endif
    }

    private func setPath(_ value: [CatalogItem]) {
        failedOpen = nil
        navigationGeneration += 1
        navigationTask?.cancel()
        path = value
    }

    private func reload() {
        failedOpen = nil
        navigationGeneration += 1
        navigationTask?.cancel()
        catalog.invalidate()
        refresh += 1
    }

    private func go(_ destination: String) {
        failedOpen = nil
        navigationGeneration += 1
        navigationTask?.cancel()
        if destination == "Home" { library = "all" }
        screen = destination
        path = []
        status = ""
    }

    private func open(_ item: CatalogItem) {
        let row = path.isEmpty && screen == "Home" ? catalog.rows.firstIndex(where: { $0.items.contains(where: { $0.id == item.id }) }) : nil
        catalog.navigationMemory.remember(viewKey, anchor: row.map { "home.row.\($0)" } ?? item.id, focus: "card.\(item.id)")
        failedOpen = nil
        catalog.error = ""
        navigationGeneration += 1
        let current = navigationGeneration
        navigationTask?.cancel()
        navigationTask = Task {
            do {
                let ancestors = try await catalog.ancestors(for: item.id)
                try Task.checkCancellation()
                guard current == navigationGeneration else { return }
                setPath(ancestors)
            } catch {
                if !Task.isCancelled && current == navigationGeneration {
                    failedOpen = item
                    catalog.error = "Unable to open title: \(error.localizedDescription)"
                }
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
        settings = false
        options.source = "all"
        options.titleOnly = false
        options.methods = Set(catalog.capabilities?.methods.filter { $0 != "title_only" } ?? ["text", "fuzzy", "semantic"])
        saveSettings()
        go("Search")
    }

    private func menuButton(_ title: String, _ icon: String, libraryID: String? = nil, action: @escaping () -> Void) -> some View {
        let selected = (screen == "Home" && title == "Home") || (screen == "Search" && title == "Search titles") || (screen == "Jellyfin profile" && title == "Jellyfin profile") || (screen == "Library" && libraryID == library)
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
            menuButton("Jellyfin profile", "person.crop.circle") { go("Jellyfin profile") }
            if expanded { Text("Libraries").font(.caption).foregroundStyle(.secondary).padding(.top, 12) }
            ForEach(catalog.libraries, id: \.id) { value in
                menuButton(value.title, "rectangle.stack", libraryID: value.id) { library = value.id; go("Library") }
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
                    menuButton("Jellyfin profile", "person.crop.circle") { go("Jellyfin profile") }
                    if expanded {
                        Text("Libraries").font(.system(size: 18, weight: .medium)).foregroundStyle(.secondary)
                            .padding(.horizontal, 12).padding(.top, 14).padding(.bottom, 4)
                    }
                    ForEach(catalog.libraries, id: \.id) { value in
                        menuButton(value.title, "rectangle.stack", libraryID: value.id) { library = value.id; go("Library") }
                    }
                }
            }
            if expanded {
                VStack(alignment: .leading, spacing: 6) {
                    Divider().padding(.bottom, 6)
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

    private var viewKey: CatalogNavigationKey {
        CatalogNavigationKey(screen: screen, library: library, itemID: path.last?.id ?? "",
            query: screen == "Search" ? query : "", source: options.source,
            types: options.types.sorted(), methods: options.methods.sorted(), titleOnly: options.titleOnly,
            sourceID: catalog.scope.sourceId)
    }

    private func content(compact: Bool, width: CGFloat) -> some View {
        ScrollViewReader { restorationReader in
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                if !status.isEmpty { Text(status).foregroundStyle(.secondary).accessibilityIdentifier("refresh.status") }
                if !catalog.persistenceWarning.isEmpty {
                    Text(catalog.persistenceWarning).font(.caption).foregroundStyle(.secondary).accessibilityIdentifier("settings.warning")
                    Button("Retry saving settings") { catalog.retrySettingsSave() }.accessibilityIdentifier("settings.retry")
                }
                if !catalog.error.isEmpty {
                    Text(catalog.error).accessibilityIdentifier("core.error")
                    Button("Retry") {
                        if let item = failedOpen { open(item) } else { reload() }
                    }.accessibilityIdentifier("core.retry")
                }
                if catalog.loading { ProgressView("Loading catalog…").accessibilityIdentifier("core.loading") }
                if !path.isEmpty {
                    breadcrumbs
                    if let item = catalog.detail, item.id == path.last?.id {
                        detail(item, width: width, compact: compact)
                    }
                } else if screen == "Home" {
                    heading("Home")
                    Button("Browse libraries") { library = "all"; go("Library") }
                    Text("Catalog membership, not playback availability.").font(.caption).foregroundStyle(.secondary)
                    ForEach(Array(catalog.rows.enumerated()), id: \.offset) { index, row in
                        VStack(alignment: .leading, spacing: 14) {
                            Text(row.title).font(.title3.weight(.semibold)).accessibilityIdentifier("home.row.\(index)")
                            if let failure = row.readError {
                                Text(homeRowError(failure.code)).font(.caption).foregroundStyle(.secondary)
                                Button("Retry \(row.title)") { Task { await catalog.retryHomeRows() } }
                            }
                            if row.items.isEmpty && row.readError == nil && !catalog.loading {
                                Text("No titles in this row.").foregroundStyle(.secondary)
                            }
                            ScrollViewReader { reader in
                                ScrollView(.horizontal) {
                                    HStack(alignment: .top, spacing: 18) {
                                        ForEach(row.items.map(CatalogItem.init)) { item in card(item, width: compact ? 140 : ScreenroomTheme.posterWidth) }
                                    }.padding(.vertical, 8)
                                }
                                .onAppear {
                                    if let focus = catalog.navigationMemory.focus(for: viewKey), focus.hasPrefix("card.") {
                                        reader.scrollTo(String(focus.dropFirst(5)), anchor: .center)
                                    }
                                }
                            }
                        }.id("home.row.\(index)")
                    }
                    if catalog.rows.isEmpty && !catalog.loading && catalog.error.isEmpty {
                        Text("No home rows are available. Browse libraries or refresh the catalog.").foregroundStyle(.secondary)
                    }
                } else if screen == "Library" {
                    heading(catalog.libraryTitle(library))
                    Text(loadedItemCount(catalog.items.count)).foregroundStyle(.secondary).accessibilityIdentifier("catalog.loadedCount")
                    grid(catalog.items, compact: compact)
                    if catalog.items.isEmpty && !catalog.loading && catalog.error.isEmpty {
                        Text("No titles in this library.").foregroundStyle(.secondary)
                    }
                    loadMore(search: false)
                } else if screen == "Search" {
                    search(compact: compact)
                } else if screen == "Jellyfin profile" {
                    heading("Jellyfin profile")
                    JellyfinProfileView(profile: catalog.jellyfinProfile)
                }
             }
             .padding(compact ? 16 : ScreenroomTheme.inset)
             .scrollTargetLayout()
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollPosition(id: Binding(get: { catalog.navigationMemory.anchor(for: viewKey) }, set: { catalog.navigationMemory.remember(viewKey, anchor: $0) }))
        .onChange(of: catalog.loading) { _, loading in
            if !loading { navigationFocus = catalog.navigationMemory.focus(for: viewKey) }
        }
        .onChange(of: catalog.navigationRestorationRevision) { _, revision in
            let key = viewKey
            Task { @MainActor in
                await Task.yield()
                guard revision == catalog.navigationRestorationRevision, key == viewKey else { return }
                if let anchor = catalog.navigationMemory.anchor(for: key) {
                    restorationReader.scrollTo(anchor, anchor: .top)
                }
                navigationFocus = catalog.navigationMemory.focus(for: key)
            }
        }
        #if os(tvOS)
        .focusSection()
        #endif
        }
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

    private func card(_ item: CatalogItem, width: CGFloat, seasonContext: Bool = false) -> some View {
        let labels = item.labels(seasonContext: seasonContext)
        return Button { open(item) } label: {
            VStack(alignment: .leading, spacing: 8) {
                PosterView(item: item, seasonContext: seasonContext).frame(width: width, height: width * 1.5).accessibilityHidden(true)
                HStack(alignment: .firstTextBaseline, spacing: 5) {
                    if !labels.primary.isEmpty { Text(labels.primary).font(.body.weight(.medium)).lineLimit(2) }
                    if !labels.number.isEmpty { Text(labels.number).font(.caption).foregroundStyle(.secondary) }
                }
                Text(labels.subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(2)
            }.frame(width: width, alignment: .leading).contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .id(item.id)
        .focused($navigationFocus, equals: "card.\(item.id)")
        .accessibilityLabel([labels.heading, labels.subtitle, item.playbackAccessibility].filter { !$0.isEmpty }.joined(separator: ", "))
        .accessibilityValue(item.artwork == nil ? "Designed placeholder; poster not included" : catalog.loadedArtwork.contains(item.artwork!.url) ? "Core artwork ready" : "Artwork loading")
        .accessibilityIdentifier("card.\(item.id)")
    }

    private func grid(_ items: [CatalogItem], compact: Bool, parent: CatalogItem? = nil) -> some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: compact ? 140 : ScreenroomTheme.posterWidth), spacing: 20)], alignment: .leading, spacing: 24) {
            ForEach(items) { item in card(item, width: compact ? 140 : ScreenroomTheme.posterWidth, seasonContext: item.isSeasonChild(of: parent)) }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .scrollTargetLayout()
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
                if item.kind == "Series" || item.kind == "Season" {
                    Text(loadedItemCount(catalog.children.count)).foregroundStyle(.secondary).accessibilityIdentifier("catalog.loadedCount")
                    grid(catalog.children, compact: compact, parent: item)
                    if catalog.children.isEmpty && !catalog.loading && catalog.error.isEmpty {
                        Text(item.kind == "Series" ? "No seasons are available." : "No episodes are available.").foregroundStyle(.secondary)
                    }
                    loadMore(search: false)
                }
                PeopleView(title: "Cast", people: catalog.credits.filter { $0.department.lowercased() == "cast" })
                PeopleView(title: "Crew", people: catalog.credits.filter { $0.department.lowercased() != "cast" })
            }
        }.frame(maxWidth: ScreenroomTheme.detailMaxWidth, alignment: .leading)
    }

    private func detailSummary(_ item: CatalogItem) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("\(item.kind) · \(String(item.year))").foregroundStyle(.secondary)
            heading(item.title)
            Text(item.meta).foregroundStyle(.secondary)
            if item.kind == "Movie" || item.kind == "Episode" {
                Button("Play", systemImage: "play.fill") {
                    guard let client = catalog.client else { return }
                    catalog.navigationMemory.remember(viewKey, focus: "detail.play")
                    playbackTitle = item.title
                    playback = NativePlayback(client: client, itemID: item.id, scope: catalog.scope)
                }
                .disabled(catalog.client == nil)
                .accessibilityIdentifier("detail.play")
                .focused($navigationFocus, equals: "detail.play")
            }
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

    private func loadMore(search: Bool) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            if !catalog.pageError.isEmpty { Text(catalog.pageError).font(.caption).foregroundStyle(.secondary).accessibilityIdentifier("catalog.pageError") }
            if !(search ? catalog.searchPage.token : catalog.itemPage.token).isEmpty {
                Button(catalog.pageError.isEmpty ? "Load more" : "Retry loading more") { Task { await catalog.loadMore(search: search) } }
                    .frame(minHeight: ScreenroomTheme.controlSize)
                    .disabled(catalog.pageLoading)
                    .accessibilityIdentifier("catalog.loadMore")
            }
            if catalog.pageLoading { ProgressView("Loading more…") }
        }
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
                               selection(choiceLabel(source), selected: options.source == source) { options.source = source; saveSettings() }
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
                         saveSettings()
                     } }
                     ForEach(catalog.capabilities?.methods.filter { $0 != "title_only" } ?? [], id: \.self) { method in
                          selection(choiceLabel(method), selected: options.methods.contains(method), disabled: (method == "semantic" && options.titleOnly) || (options.methods.contains(method) && (options.methods.count == 1 || (["text", "fuzzy"].contains(method) && options.methods.intersection(["text", "fuzzy"]).count == 1)))) {
                             if options.methods.contains(method) { options.methods.remove(method) } else { options.methods.insert(method) }
                             saveSettings()
                         }.accessibilityIdentifier("search.method.\(choiceLabel(method))")
                    }
                     if let description = catalog.capabilities?.semanticDescription { Text(description).font(.caption).foregroundStyle(.secondary) }
                }.padding(20).background(ScreenroomTheme.sidebar, in: RoundedRectangle(cornerRadius: 12))
            }
             let results = catalog.results
             if !catalog.searchDegradation.isEmpty { Text(catalog.searchDegradation).font(.caption).foregroundStyle(.secondary).accessibilityIdentifier("search.degradation") }
             if let effective = catalog.effectiveSearchMethods, !query.isEmpty, query == settled {
                 Text("Effective methods: \([(effective.text, "Text"), (effective.fuzzy, "Fuzzy"), (effective.semantic, "Semantic")].filter { $0.0 }.map { $0.1 }.joined(separator: ", "))")
                     .font(.caption).foregroundStyle(.secondary).accessibilityIdentifier("search.effectiveMethods")
             }
              Text(query.trimmingCharacters(in: .whitespaces).isEmpty ? "Enter a title to search \(catalog.libraryTitle(library))." : query != settled ? "Searching titles…" : results.isEmpty ? "No matches in \(catalog.libraryTitle(library)). Try another search." : "\(results.count) \(results.count == 1 ? "result" : "results") in \(catalog.libraryTitle(library))")
                .foregroundStyle(.secondary).accessibilityIdentifier("search.status")
            if !query.isEmpty && query == settled { grid(results, compact: compact); loadMore(search: true) }
        }
    }

    private var typeChoices: some View {
        ForEach(catalog.capabilities?.types.map(searchType) ?? [], id: \.self) { kind in
            selection(choiceLabel(kind), selected: options.types.contains(kind), disabled: options.types.contains(kind) && options.types.count == 1) {
                  if options.types.contains(kind) { options.types.remove(kind) } else { options.types.insert(kind) }
                  saveSettings()
            }.accessibilityIdentifier("search.type.\(kind)")
        }
    }
}

struct PosterView: View {
    let item: CatalogItem
    var seasonContext = false

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
                 Text(item.labels(seasonContext: seasonContext).heading)
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
            .overlay(alignment: .topTrailing) {
                if item.watched {
                    Image(systemName: "checkmark")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Color(hex: "e3f4e9"))
                        .frame(width: 26, height: 26)
                        .background(Color(hex: "16251f").opacity(0.93), in: Circle())
                        .padding(8).accessibilityHidden(true)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 10))
        }.accessibilityElement(children: .ignore)
            .accessibilityLabel([item.labels(seasonContext: seasonContext).heading, item.kind == "Episode" ? item.title : "", "Poster", item.playbackAccessibility].filter { !$0.isEmpty }.joined(separator: ", "))
    }
}
