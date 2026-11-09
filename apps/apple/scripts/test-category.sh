#!/bin/bash
configure_apple_test_category() {
    apple_test_category=${E2E_CATEGORY-all}
    case "$apple_test_category" in
        happy-path) apple_test_methods=(testHappyPathShowcase) ;;
        retry) apple_test_methods=(testCoreUnavailableAndRetry) ;;
        regression) apple_test_methods=(testHomeCatalogAndNavigation testFreshSearchAndHomeScope testSearchPaginationAndPreferencesRoundTrip) ;;
        all) apple_test_methods=(testHappyPathShowcase testCoreUnavailableAndRetry testHomeCatalogAndNavigation testFreshSearchAndHomeScope testSearchPaginationAndPreferencesRoundTrip) ;;
        *) echo 'E2E_CATEGORY must be happy-path, retry, regression, or all' >&2; return 2 ;;
    esac
}
