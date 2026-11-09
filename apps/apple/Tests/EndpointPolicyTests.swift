import Foundation

@main
struct EndpointPolicyTests {
    @MainActor
    static func main() async {
        let allowed = ProcessInfo.processInfo.environment["EXPECT_ENDPOINT_ALLOWED"] == "1"
        let store = CatalogStore()
        precondition((store.client != nil) == allowed, "Unexpected endpoint policy result")
        if !allowed {
            precondition(!store.error.isEmpty)
            await store.load(screen: "Home", library: "All libraries", itemID: nil)
            precondition(!store.error.isEmpty && !store.loading && store.rows.isEmpty)
        }
        print("Endpoint policy passed")
    }
}
