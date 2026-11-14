package org.screenroom.ui

import android.content.Context
import android.content.SharedPreferences
import org.json.JSONObject
import org.screenroom.core.*

class SettingsStore(context: Context, namespace: String, private val commit: (SharedPreferences.Editor) -> Boolean = { it.commit() }) {
    private val disk = context.getSharedPreferences("screenroom-settings-$namespace", Context.MODE_PRIVATE)
    val session by lazy { LocalSettings(load(), ::checked) { coreRequest { save(it) }; Unit } }
    fun load(): ClientSettings = try {
        val json = JSONObject(disk.getString("settings", null) ?: error("Missing settings"))
        check(json.getInt("version") == 1)
        normalizeSavedSettings(withTitleOnlyConstraint(ClientSettings(
            SourceLibraryScope(json.getString("sourceId"), json.getString("libraryId")),
            json.getString("source"),
            SearchTypes(json.getBoolean("movie"), json.getBoolean("series"), json.getBoolean("episode")),
            SearchMethods(json.getBoolean("titleOnly"), json.getBoolean("text"), json.getBoolean("fuzzy"), json.getBoolean("semantic")))))
    } catch (_: Exception) { defaultSettings() }

    fun save(settings: ClientSettings): ClientSettings {
        val value = checked(settings)
        val scope = value.scope ?: SourceLibraryScope("", "")
        val types = requireNotNull(value.types)
        val methods = requireNotNull(value.methods)
        val json = JSONObject().put("version", 1).put("sourceId", scope.sourceId).put("libraryId", scope.libraryId)
            .put("source", value.source).put("movie", types.movie).put("series", types.series).put("episode", types.episode)
            .put("titleOnly", methods.titleOnly).put("text", methods.text).put("fuzzy", methods.fuzzy).put("semantic", methods.semantic)
        check(commit(disk.edit().putString("settings", json.toString()))) { "Unable to persist settings" }
        return value
    }

    fun checked(settings: ClientSettings): ClientSettings = validateSettings(withTitleOnlyConstraint(settings))

    private fun withTitleOnlyConstraint(settings: ClientSettings): ClientSettings {
        val methods = settings.methods
        return if (methods?.titleOnly == true) settings.copy(methods = methods.copy(semantic = false)) else settings
    }
}
