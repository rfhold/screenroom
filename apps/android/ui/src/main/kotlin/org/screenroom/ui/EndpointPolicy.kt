package org.screenroom.ui

import java.net.URI

internal fun endpointError(endpoint: String, debug: Boolean): String? {
    val uri = try { URI(endpoint) } catch (_: Exception) { return "Invalid API endpoint." }
    val scheme = uri.scheme?.lowercase()
    if (endpoint != endpoint.trim() || uri.host.isNullOrEmpty() || uri.rawUserInfo != null ||
        uri.rawFragment != null || uri.rawQuery != null || uri.rawPath !in listOf("", "/") ||
        uri.port !in -1..65535 || uri.port == 0) return "Invalid API endpoint. Use an origin without credentials or a path."
    if (scheme == "https") return null
    if (debug && scheme == "http" && uri.host.lowercase() in setOf("127.0.0.1", "localhost")) return null
    return "HTTPS API endpoint required. Debug HTTP is limited to 127.0.0.1 and localhost."
}
