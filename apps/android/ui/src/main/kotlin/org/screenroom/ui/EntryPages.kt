package org.screenroom.ui

/** In-memory reads owned by a bounded navigation entry, never saved across clients. */
internal class EntryPages<T>(private val id: (T) -> String, private val limit: Int = 32) {
    private data class Read<T>(val key: Any, val pages: IncrementalPages<T>)
    private val entries = linkedMapOf<String, Read<T>>()
    fun get(entry: String, key: Any): IncrementalPages<T> {
        entries[entry]?.takeIf { it.key == key }?.let { return it.pages }
        return IncrementalPages(id).also {
            entries.remove(entry)
            entries[entry] = Read(key, it)
            while (entries.size > limit) entries.remove(entries.keys.first())
        }
    }
    fun remove(entry: String) { entries.remove(entry) }
}
