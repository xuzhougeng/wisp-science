package science.wisp.android

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import science.wisp.remote.*
import kotlinx.serialization.json.Json

/** Only ciphertext is stored on disk; the AES key stays in Android Keystore. */
class DeviceCredentials(context: Context, private val alias: String = "science.wisp.remote.v1") {
    private val file = AtomicFile(File(context.noBackupFilesDir, "remote-credentials.v1"))
    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(alias, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build())
        }.generateKey()
    }
    fun load(): RemoteConfiguration? {
        if (!hasSavedFile(file)) return null
        val bytes = file.readFully()
        require(bytes.size >= 28)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes.copyOfRange(0, 12)))
        cipher.updateAAD(alias.toByteArray())
        val value = Json.parseToJsonElement(String(cipher.doFinal(bytes.copyOfRange(12, bytes.size)), Charsets.UTF_8))
        return RemoteConfiguration.parse(value.field("relay").text(), value.field("code").text())
    }
    fun save(configuration: RemoteConfiguration) {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key())
        cipher.updateAAD(alias.toByteArray())
        val bytes = obj("relay" to string(configuration.relay.toString()), "code" to string(configuration.code)).toString().toByteArray()
        writeAtomically(file, cipher.iv + cipher.doFinal(bytes))
    }
    fun clear() {
        file.delete()
        KeyStore.getInstance("AndroidKeyStore").apply { load(null); deleteEntry(alias) }
    }
}

class DeviceEntries(context: Context) {
    private val file = AtomicFile(File(context.noBackupFilesDir, "research-text.v1.json"))
    fun load(): List<LocalEntry> = if (hasSavedFile(file)) LocalEntries.decode(String(file.readFully(), Charsets.UTF_8)) else emptyList()
    fun save(entries: List<LocalEntry>) = writeAtomically(file, LocalEntries.encode(entries).toByteArray())
}

// API 26's AtomicFile can recover a .bak even when a crash removed the base file.
private fun hasSavedFile(file: AtomicFile) = file.baseFile.exists() || File(file.baseFile.path + ".bak").exists()

private fun writeAtomically(file: AtomicFile, bytes: ByteArray) {
    val stream = file.startWrite()
    try { stream.write(bytes); file.finishWrite(stream) }
    catch (error: Exception) { file.failWrite(stream); throw error }
}
