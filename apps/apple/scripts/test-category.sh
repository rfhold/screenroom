#!/bin/bash
configure_apple_test_category() {
    apple_test_journey=${SCREENROOM_E2E_JOURNEY-browse}
    apple_test_category=${E2E_CATEGORY-all}
    apple_test_class=HomeE2ETests
    case "$apple_test_journey" in
        browse) ;;
        playback)
            if [ "$apple_test_category" != happy-path ]; then
                echo 'Playback journey requires E2E_CATEGORY=happy-path' >&2; return 2
            fi
            apple_test_class=PlayerScreenshots
            apple_test_methods=(testNosferatuPlayerStates)
            return 0 ;;
        *) echo 'SCREENROOM_E2E_JOURNEY must be browse or playback' >&2; return 2 ;;
    esac
    case "$apple_test_category" in
        happy-path) apple_test_methods=(testHappyPathShowcase) ;;
        retry) apple_test_methods=(testCoreUnavailableAndRetry) ;;
        regression) apple_test_methods=(testHomeCatalogAndNavigation testFreshSearchAndHomeScope testSearchPaginationAndPreferencesRoundTrip) ;;
        all) apple_test_methods=(testHappyPathShowcase testCoreUnavailableAndRetry testHomeCatalogAndNavigation testFreshSearchAndHomeScope testSearchPaginationAndPreferencesRoundTrip) ;;
        *) echo 'E2E_CATEGORY must be happy-path, retry, regression, or all' >&2; return 2 ;;
    esac
}
