package app.stiksound.android

import android.app.Activity
import android.content.ContentValues
import android.content.Intent
import android.os.Environment
import android.provider.MediaStore
import androidx.core.content.FileProvider
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

@InvokeArg
class ShareArgs {
  lateinit var path: String
  lateinit var mimeType: String
}

@InvokeArg
class SaveArgs {
  lateinit var path: String
  lateinit var fileName: String
  lateinit var mimeType: String
}

/** Share sheet + save to Downloads/StikSound. Files arrive already written to cache/share/ by Rust. */
@TauriPlugin
class StikSharePlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun share(invoke: Invoke) {
    val args = invoke.parseArgs(ShareArgs::class.java)
    try {
      val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", File(args.path))
      val send = Intent(Intent.ACTION_SEND).apply {
        type = args.mimeType
        putExtra(Intent.EXTRA_STREAM, uri)
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      }
      activity.runOnUiThread {
        activity.startActivity(Intent.createChooser(send, null).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
      }
      invoke.resolve()
    } catch (e: Exception) {
      invoke.reject("share-failed: ${e.message}")
    }
  }

  @Command
  fun save(invoke: Invoke) {
    val args = invoke.parseArgs(SaveArgs::class.java)
    val source = File(args.path)
    try {
      val values = ContentValues().apply {
        put(MediaStore.MediaColumns.DISPLAY_NAME, args.fileName)
        put(MediaStore.MediaColumns.MIME_TYPE, args.mimeType)
        put(MediaStore.MediaColumns.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/StikSound")
      }
      val resolver = activity.contentResolver
      val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
        ?: return invoke.reject("save-failed: MediaStore insert returned null")
      resolver.openOutputStream(uri).use { out ->
        requireNotNull(out) { "no output stream" }
        source.inputStream().use { it.copyTo(out) }
      }
      invoke.resolve(JSObject().put("name", args.fileName))
    } catch (e: Exception) {
      invoke.reject("save-failed: ${e.message}")
    } finally {
      source.delete()
    }
  }
}
