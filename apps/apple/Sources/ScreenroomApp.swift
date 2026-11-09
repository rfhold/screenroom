import SwiftUI

@main
struct ScreenroomApp: App {
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
