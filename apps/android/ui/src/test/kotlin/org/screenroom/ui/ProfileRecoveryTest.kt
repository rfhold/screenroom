package org.screenroom.ui

import org.junit.Assert.*
import org.junit.Test

class ProfileRecoveryTest {
    @Test fun reloadDoesNotDisownRecoveryButProfileRevisionRejectsOldAttachmentAndActions() {
        val recovery = ProfileRecovery<Any>()
        val old = Any()
        val owner = recovery.capture()
        assertTrue(recovery.retain(owner, old))
        ReadRefresh().advance()
        assertSame(old, recovery.visible)
        assertTrue(recovery.owns(owner, old))
        recovery.replaceProfile()
        assertNull(recovery.visible)
        assertFalse(recovery.owns(owner, old))
        val current = Any()
        assertTrue(recovery.retain(recovery.capture(), current))
        assertFalse(recovery.retain(owner, old))
        assertSame(current, recovery.visible)
    }
}
