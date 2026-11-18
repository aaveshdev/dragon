package org.dragonlang.runtime

import android.Manifest
import android.app.DatePickerDialog
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.TimePickerDialog
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.location.LocationManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.VibrationEffect
import android.os.Vibrator
import android.provider.OpenableColumns
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.app.ActivityCompat
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.util.Calendar
import java.util.concurrent.Executors

class DragonPlatform(private val activity: ComponentActivity, private val reply: (String, Any?) -> Unit) {
    private val io = Executors.newCachedThreadPool()
    private val main = Handler(Looper.getMainLooper())
    private var seq = 0
    val plugins: List<DragonPlugin> by lazy {
        DragonPlugins.create(PluginContext(activity, main, File(activity.filesDir, "dragon_docs")))
    }

    private fun key(prefix: String): String = "dragon_" + prefix + "_" + (seq++)

    fun handle(call: JSONObject) {
        val id = call.optString("id")
        val method = call.optString("method")
        val args = call.optJSONObject("args") ?: JSONObject()
        try {
            when (method) {
                "http" -> http(id, args)
                "clipboard.set" -> {
                    val cm = activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                    cm.setPrimaryClip(ClipData.newPlainText("dragon", args.optString("text")))
                }
                "clipboard.get" -> {
                    val cm = activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                    val t = cm.primaryClip?.getItemAt(0)?.coerceToText(activity)?.toString() ?: ""
                    reply(id, JSONObject().put("text", t))
                }
                "toast" -> Toast.makeText(
                    activity, args.optString("message"),
                    if (args.optBoolean("long")) Toast.LENGTH_LONG else Toast.LENGTH_SHORT
                ).show()
                "openUrl" -> {
                    val i = Intent(Intent.ACTION_VIEW, Uri.parse(args.optString("url")))
                    i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                    activity.startActivity(i)
                    reply(id, true)
                }
                "share" -> {
                    val i = Intent(Intent.ACTION_SEND)
                    i.type = "text/plain"
                    i.putExtra(Intent.EXTRA_TEXT, args.optString("text"))
                    if (args.has("subject")) i.putExtra(Intent.EXTRA_SUBJECT, args.optString("subject"))
                    activity.startActivity(Intent.createChooser(i, null))
                }
                "haptic", "vibrate" -> vibrate(if (method == "vibrate") args.optLong("ms", 40) else 20L)
                "deviceInfo" -> reply(
                    id, JSONObject()
                        .put("platform", "android")
                        .put("model", Build.MODEL)
                        .put("manufacturer", Build.MANUFACTURER)
                        .put("sdk", Build.VERSION.SDK_INT)
                        .put("release", Build.VERSION.RELEASE)
                )
                "permission.check" -> reply(id, JSONObject().put("granted", hasPermission(args.optString("name"))))
                "permission.request" -> requestPermission(id, args.optString("name"))
                "notify" -> notify(args)
                "notify.cancel" -> (activity.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager)
                    .cancel(args.optInt("id", 1))
                "datePicker" -> datePicker(id, args.optString("initial"))
                "timePicker" -> timePicker(id, args.optString("initial"))
                "pickFile" -> pickFile(id, args.optString("mime", "*/*"))
                "takePhoto" -> takePhoto(id)
                "location" -> location(id)
                "statusBar" -> { }
                "orientation" -> {
                    activity.requestedOrientation = when (args.optString("mode")) {
                        "portrait" -> ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
                        "landscape" -> ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE
                        else -> ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
                    }
                }
                "exit" -> activity.finish()
                else -> {
                    val handled = plugins.any { it.handles(method) && it.handle(method, args) { r -> reply(id, r) } }
                    if (!handled) reply(id, null)
                }
            }
        } catch (e: Exception) {
            reply(id, JSONObject().put("error", e.toString()))
        }
    }

    private fun http(id: String, a: JSONObject) {
        io.execute {
            val out = JSONObject()
            try {
                val conn = URL(Uri.encode(a.optString("url"), "%:/?#[]@!\$&'()*+,;=-._~")).openConnection() as HttpURLConnection
                if (a.optBoolean("insecure") && conn is javax.net.ssl.HttpsURLConnection) {
                    val trustAll = arrayOf<javax.net.ssl.TrustManager>(object : javax.net.ssl.X509TrustManager {
                        override fun checkClientTrusted(c: Array<java.security.cert.X509Certificate>, t: String) {}
                        override fun checkServerTrusted(c: Array<java.security.cert.X509Certificate>, t: String) {}
                        override fun getAcceptedIssuers(): Array<java.security.cert.X509Certificate> = arrayOf()
                    })
                    val ctx = javax.net.ssl.SSLContext.getInstance("TLS")
                    ctx.init(null, trustAll, java.security.SecureRandom())
                    conn.sslSocketFactory = ctx.socketFactory
                    conn.hostnameVerifier = javax.net.ssl.HostnameVerifier { _, _ -> true }
                }
                conn.requestMethod = a.optString("method", "GET")
                val t = a.optInt("timeout", 30000)
                conn.connectTimeout = t
                conn.readTimeout = t
                val hs = a.optJSONObject("headers")
                if (hs != null) {
                    val it = hs.keys()
                    while (it.hasNext()) {
                        val k = it.next()
                        conn.setRequestProperty(k, hs.optString(k))
                    }
                }
                if (a.has("bodyParts") && conn.requestMethod != "GET") {
                    conn.doOutput = true
                    val parts = a.getJSONArray("bodyParts")
                    conn.outputStream.use { os ->
                        for (i in 0 until parts.length()) {
                            val p = parts.getJSONObject(i)
                            if (p.has("text")) os.write(p.getString("text").toByteArray(Charsets.UTF_8))
                            else if (p.has("base64")) os.write(android.util.Base64.decode(p.getString("base64"), android.util.Base64.DEFAULT))
                        }
                    }
                } else if (a.has("body") && conn.requestMethod != "GET") {
                    conn.doOutput = true
                    conn.outputStream.use { it.write(a.optString("body").toByteArray()) }
                }
                val code = conn.responseCode
                val stream = if (code in 200..399) conn.inputStream else conn.errorStream
                val body = stream?.bufferedReader()?.use { it.readText() } ?: ""
                val hdrs = JSONObject()
                for ((k, v) in conn.headerFields) {
                    if (k != null && v != null && v.isNotEmpty()) hdrs.put(k.lowercase(), v[0])
                }
                out.put("status", code).put("body", body).put("headers", hdrs)
            } catch (e: Exception) {
                out.put("status", 0).put("error", e.toString())
            }
            reply(id, out)
        }
    }

    private fun vibrate(ms: Long) {
        @Suppress("DEPRECATION")
        val v = activity.getSystemService(Context.VIBRATOR_SERVICE) as? Vibrator ?: return
        if (Build.VERSION.SDK_INT >= 26) v.vibrate(VibrationEffect.createOneShot(ms, VibrationEffect.DEFAULT_AMPLITUDE))
        else @Suppress("DEPRECATION") v.vibrate(ms)
    }

    private fun androidPermission(name: String): String? = when (name) {
        "camera" -> Manifest.permission.CAMERA
        "microphone" -> Manifest.permission.RECORD_AUDIO
        "location" -> Manifest.permission.ACCESS_FINE_LOCATION
        "contacts" -> Manifest.permission.READ_CONTACTS
        "storage" -> if (Build.VERSION.SDK_INT >= 33) Manifest.permission.READ_MEDIA_IMAGES else Manifest.permission.READ_EXTERNAL_STORAGE
        "audio" -> if (Build.VERSION.SDK_INT >= 33) Manifest.permission.READ_MEDIA_AUDIO else Manifest.permission.READ_EXTERNAL_STORAGE
        "notifications" -> if (Build.VERSION.SDK_INT >= 33) Manifest.permission.POST_NOTIFICATIONS else null
        else -> name
    }

    private fun hasPermission(name: String): Boolean {
        val p = androidPermission(name) ?: return true
        return ContextCompat.checkSelfPermission(activity, p) == PackageManager.PERMISSION_GRANTED
    }

    private fun requestPermission(id: String, name: String) {
        val p = androidPermission(name)
        if (p == null || hasPermission(name)) {
            reply(id, JSONObject().put("granted", true))
            return
        }
        main.post {
            val launcher = activity.activityResultRegistry.register(
                key("perm"), ActivityResultContracts.RequestPermission()
            ) { granted -> reply(id, JSONObject().put("granted", granted)) }
            launcher.launch(p)
        }
    }

    private fun notify(a: JSONObject) {
        val nm = activity.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= 26) {
            nm.createNotificationChannel(NotificationChannel("dragon", "Dragon", NotificationManager.IMPORTANCE_DEFAULT))
        }
        if (!hasPermission("notifications")) return
        val n = NotificationCompat.Builder(activity, "dragon")
            .setSmallIcon(activity.applicationInfo.icon)
            .setContentTitle(a.optString("title"))
            .setContentText(a.optString("body"))
            .setAutoCancel(true)
            .build()
        nm.notify(a.optInt("id", 1), n)
    }

    private fun datePicker(id: String, initial: String) {
        val c = Calendar.getInstance()
        val p = initial.split("-")
        if (p.size == 3) {
            c.set(p[0].toIntOrNull() ?: c.get(Calendar.YEAR), (p[1].toIntOrNull() ?: 1) - 1, p[2].toIntOrNull() ?: 1)
        }
        main.post {
            val d = DatePickerDialog(activity, { _, y, m, day ->
                reply(id, JSONObject().put("date", String.format("%04d-%02d-%02d", y, m + 1, day)))
            }, c.get(Calendar.YEAR), c.get(Calendar.MONTH), c.get(Calendar.DAY_OF_MONTH))
            d.setOnCancelListener { reply(id, null) }
            d.show()
        }
    }

    private fun timePicker(id: String, initial: String) {
        val p = initial.split(":")
        val h = p.getOrNull(0)?.toIntOrNull() ?: 12
        val m = p.getOrNull(1)?.toIntOrNull() ?: 0
        main.post {
            val d = TimePickerDialog(activity, { _, hh, mm ->
                reply(id, JSONObject().put("time", String.format("%02d:%02d", hh, mm)))
            }, h, m, true)
            d.setOnCancelListener { reply(id, null) }
            d.show()
        }
    }

    private fun pickFile(id: String, mime: String) {
        main.post {
            val launcher = activity.activityResultRegistry.register(
                key("pick"), ActivityResultContracts.OpenDocument()
            ) { uri ->
                if (uri == null) {
                    reply(id, null)
                } else {
                    io.execute {
                        try {
                            var name = "file"
                            activity.contentResolver.query(uri, null, null, null, null)?.use { c ->
                                val i = c.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                                if (c.moveToFirst() && i >= 0) name = c.getString(i)
                            }
                            val out = File(activity.cacheDir, "picked_" + System.currentTimeMillis() + "_" + name)
                            activity.contentResolver.openInputStream(uri)?.use { input ->
                                out.outputStream().use { input.copyTo(it) }
                            }
                            reply(id, JSONObject().put("name", name).put("path", out.absolutePath).put("uri", uri.toString()))
                        } catch (e: Exception) {
                            reply(id, null)
                        }
                    }
                }
            }
            launcher.launch(arrayOf(mime))
        }
    }

    private fun takePhoto(id: String) {
        main.post {
            val launcher = activity.activityResultRegistry.register(
                key("photo"), ActivityResultContracts.TakePicturePreview()
            ) { bmp: Bitmap? ->
                if (bmp == null) {
                    reply(id, null)
                } else {
                    io.execute {
                        val out = File(activity.cacheDir, "photo_" + System.currentTimeMillis() + ".jpg")
                        out.outputStream().use { bmp.compress(Bitmap.CompressFormat.JPEG, 90, it) }
                        reply(id, JSONObject().put("name", out.name).put("path", out.absolutePath))
                    }
                }
            }
            launcher.launch(null)
        }
    }

    @Suppress("MissingPermission")
    private fun location(id: String) {
        if (!hasPermission("location")) {
            reply(id, null)
            return
        }
        val lm = activity.getSystemService(Context.LOCATION_SERVICE) as LocationManager
        var best: android.location.Location? = null
        for (p in lm.getProviders(true)) {
            val l = lm.getLastKnownLocation(p) ?: continue
            if (best == null || l.accuracy < best.accuracy) best = l
        }
        if (best == null) reply(id, null)
        else reply(id, JSONObject().put("lat", best.latitude).put("lng", best.longitude).put("accuracy", best.accuracy.toDouble()))
    }
}
