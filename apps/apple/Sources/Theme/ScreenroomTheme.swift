import SwiftUI

enum ScreenroomTheme {
    static let content = Color(hex: "101010")
    static let sidebar = Color(hex: "171717")
    #if os(tvOS)
    static let posterWidth: CGFloat = 220
    static let inset: CGFloat = 40
    static let controlSize: CGFloat = 60
    static let detailPosterWidth: CGFloat = 240
    static let detailMaxWidth: CGFloat = 1600
    static let personWidth: CGFloat = 180
    #else
    static let posterWidth: CGFloat = 160
    static let inset: CGFloat = 24
    static let controlSize: CGFloat = 44
    static let detailPosterWidth: CGFloat = 200
    static let detailMaxWidth: CGFloat = 1120
    static let personWidth: CGFloat = 140
    #endif
}

extension Color {
    init(hex: String) {
        let value = UInt64(hex.replacingOccurrences(of: "#", with: ""), radix: 16) ?? 0
        self.init(red: Double((value >> 16) & 255) / 255, green: Double((value >> 8) & 255) / 255, blue: Double(value & 255) / 255)
    }
}
