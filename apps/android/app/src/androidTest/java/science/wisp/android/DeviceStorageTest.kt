package science.wisp.android

import android.content.ContextWrapper
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.UUID
import org.junit.Assert.*
import org.junit.Test
import science.wisp.remote.*

class DeviceStorageTest {
    @Test fun credentialsAreEncryptedAndNotesPersistAcrossStoreInstances() {
        val base = InstrumentationRegistry.getInstrumentation().targetContext
        val directory = File(base.cacheDir, "storage-test-${UUID.randomUUID()}").apply { mkdirs() }
        val context = object : ContextWrapper(base) { override fun getNoBackupFilesDir() = directory }
        val alias = "science.wisp.test.${UUID.randomUUID()}"
        val credentials = DeviceCredentials(context, alias)
        try {
            val config = RemoteConfiguration.parse("https://relay.example.test", "000102030405060708090a0b0c0d0e0f")
            credentials.save(config)
            val disk = File(directory, "remote-credentials.v1").readBytes().toString(Charsets.ISO_8859_1)
            assertFalse(disk.contains(config.code))
            assertFalse(disk.contains("relay.example.test"))
            assertEquals(config.code, DeviceCredentials(context, alias).load()!!.code)
            val entries = listOf(LocalEntry("note", "notebook", "观察", "文字记录"))
            DeviceEntries(context).save(entries)
            assertEquals(entries, DeviceEntries(context).load())
            credentials.clear()
            assertNull(credentials.load())
            assertEquals(entries, DeviceEntries(context).load())
        } finally {
            credentials.clear()
            directory.deleteRecursively()
        }
    }
}
