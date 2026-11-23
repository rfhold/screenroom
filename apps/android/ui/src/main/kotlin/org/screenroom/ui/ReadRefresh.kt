package org.screenroom.ui

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.asStateFlow

internal class ReadRefresh {
    private val epoch = MutableStateFlow(0)
    val generation = epoch.asStateFlow()
    fun advance() { epoch.value++ }
    suspend fun repeatOnRefresh(read: suspend () -> Unit) { epoch.collectLatest { read() } }
}
