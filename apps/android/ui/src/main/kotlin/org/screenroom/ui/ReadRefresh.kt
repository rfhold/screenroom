package org.screenroom.ui

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.collectLatest

internal class ReadRefresh {
    private val generation = MutableStateFlow(0)
    fun advance() { generation.value++ }
    suspend fun repeatOnRefresh(read: suspend () -> Unit) { generation.collectLatest { read() } }
}
