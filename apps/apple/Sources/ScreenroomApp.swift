import SwiftUI

@main
struct ScreenroomApp: App {
    init() {
        #if !DEBUG
        #if os(iOS)
        let platform = "ios"
        #elseif os(tvOS)
        let platform = "tvos"
        #else
        let platform = "macos"
        #endif
        let environment = Bundle.main.object(forInfoDictionaryKey: "ScreenroomTelemetryEnvironment") as? String ?? "production"
        _ = initializeClientTelemetry(platform: platform, environment: environment)
        #endif
    }

    var body: some Scene {
        WindowGroup {
            HomeView()
                #if os(macOS)
                .frame(minWidth: 320, minHeight: 480)
                #endif
        }
        #if os(macOS)
        .defaultSize(width: 1200, height: 850)
        #endif
    }
}
