package org.dragonlang.runtime

import android.content.ComponentName
import android.content.ContentUris
import android.content.Context
import android.graphics.Bitmap
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.util.Size
import androidx.core.content.ContextCompat
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.PlaybackParameters
import androidx.media3.common.Player
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import android.media.audiofx.BassBoost
import android.media.audiofx.Equalizer
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.concurrent.Executors

class DragonAudio(private val ctx: Context, private val main: Handler) {
    private val io = Executors.newFixedThreadPool(2)
    private val pending = ArrayList<(ExoPlayer) -> Unit>()
    private var connecting = false
    private var controller: MediaController? = null
    private var eq: Equalizer? = null
    private var bass: BassBoost? = null
    private var eqSession = -1
    private var sleepAt = 0L
    private val sleepRunnable = Runnable { fadeOutAndPause() }
    private var lastError: String? = null


    private fun scanFolders() {
        try {
            val paths = ArrayList<String>()
            fun walk(d: File, depth: Int) {
                val fs = d.listFiles() ?: return
                for (f in fs) {
                    if (f.isDirectory) { if (depth < 3 && !f.name.startsWith(".")) walk(f, depth + 1) }
                    else {
                        val n = f.name.lowercase()
                        if (n.endsWith(".mp3") || n.endsWith(".m4a") || n.endsWith(".flac") || n.endsWith(".ogg") ||
                            n.endsWith(".wav") || n.endsWith(".aac") || n.endsWith(".opus")) paths.add(f.absolutePath)
                    }
                }
            }
            for (dir in listOf(android.os.Environment.DIRECTORY_MUSIC, android.os.Environment.DIRECTORY_DOWNLOADS,
                android.os.Environment.DIRECTORY_DOCUMENTS, android.os.Environment.DIRECTORY_PODCASTS))
                walk(android.os.Environment.getExternalStoragePublicDirectory(dir), 0)
            if (paths.isEmpty()) return
            val latch = java.util.concurrent.CountDownLatch(paths.size)
            android.media.MediaScannerConnection.scanFile(ctx, paths.toTypedArray(), null) { _, _ -> latch.countDown() }
            latch.await(8, java.util.concurrent.TimeUnit.SECONDS)
        } catch (_: Exception) {}
    }

    fun query(reply: (Any?) -> Unit, minDurationMs: Long) {
        io.execute {
            val out = JSONArray()
            try {
                scanFolders()
                val proj = arrayOf(
                    MediaStore.Audio.Media._ID, MediaStore.Audio.Media.TITLE, MediaStore.Audio.Media.ARTIST,
                    MediaStore.Audio.Media.ALBUM, MediaStore.Audio.Media.ALBUM_ID, MediaStore.Audio.Media.DURATION,
                    MediaStore.Audio.Media.TRACK, MediaStore.Audio.Media.YEAR, MediaStore.Audio.Media.DATE_ADDED,
                    MediaStore.Audio.Media.ARTIST_ID, MediaStore.Audio.Media.SIZE, MediaStore.Audio.Media.DISPLAY_NAME
                )
                val sel = "(" + MediaStore.Audio.Media.IS_MUSIC + " != 0 OR (" + MediaStore.Audio.Media.IS_RINGTONE + " = 0 AND " +
                    MediaStore.Audio.Media.IS_NOTIFICATION + " = 0 AND " + MediaStore.Audio.Media.IS_ALARM + " = 0)) AND " +
                    MediaStore.Audio.Media.DURATION + " >= ?"
                val cur = ctx.contentResolver.query(
                    MediaStore.Audio.Media.EXTERNAL_CONTENT_URI, proj, sel,
                    arrayOf(minDurationMs.toString()), MediaStore.Audio.Media.TITLE + " COLLATE NOCASE ASC"
                )
                cur?.use { c ->
                    while (c.moveToNext()) {
                        val id = c.getLong(0)
                        val o = JSONObject()
                        o.put("id", id.toString())
                        o.put("uri", ContentUris.withAppendedId(MediaStore.Audio.Media.EXTERNAL_CONTENT_URI, id).toString())
                        var title = c.getString(1) ?: c.getString(11) ?: "Unknown"
                        o.put("title", title)
                        val artist = c.getString(2)
                        o.put("artist", if (artist == null || artist == "<unknown>") "Unknown artist" else artist)
                        val album = c.getString(3)
                        o.put("album", if (album == null || album == "<unknown>") "Unknown album" else album)
                        o.put("albumId", c.getLong(4).toString())
                        o.put("artistId", c.getLong(9).toString())
                        o.put("durationMs", c.getLong(5))
                        o.put("track", c.getInt(6))
                        o.put("year", c.getInt(7))
                        o.put("added", c.getLong(8))
                        o.put("size", c.getLong(10))
                        out.put(o)
                    }
                }
                reply(JSONObject().put("tracks", out))
            } catch (e: SecurityException) {
                reply(JSONObject().put("tracks", JSONArray()).put("error", "permission"))
            } catch (e: Exception) {
                reply(JSONObject().put("tracks", JSONArray()).put("error", e.toString()))
            }
        }
    }

    private fun artFile(albumId: Long, size: Int): File {
        val dir = File(ctx.cacheDir, "dragon_art")
        dir.mkdirs()
        return File(dir, "a${albumId}_$size.jpg")
    }

    fun art(albumId: Long, size: Int): String? {
        val f = artFile(albumId, size)
        if (f.exists() && f.length() > 0) return f.absolutePath
        if (f.exists()) return null
        try {
            var bmp: Bitmap? = null
            if (Build.VERSION.SDK_INT >= 29) {
                val uri = ContentUris.withAppendedId(MediaStore.Audio.Albums.EXTERNAL_CONTENT_URI, albumId)
                bmp = ctx.contentResolver.loadThumbnail(uri, Size(size, size), null)
            } else {
                val uri = ContentUris.withAppendedId(Uri.parse("content://media/external/audio/albumart"), albumId)
                ctx.contentResolver.openInputStream(uri)?.use { bmp = android.graphics.BitmapFactory.decodeStream(it) }
            }
            val b = bmp ?: run { f.createNewFile(); return null }
            f.outputStream().use { b.compress(Bitmap.CompressFormat.JPEG, 86, it) }
            return f.absolutePath
        } catch (e: Exception) {
            try { f.createNewFile() } catch (_: Exception) {}
            return null
        }
    }

    fun artAsync(albumId: Long, size: Int, reply: (Any?) -> Unit) {
        io.execute {
            val p = art(albumId, size)
            reply(if (p == null) null else JSONObject().put("path", p))
        }
    }


    private fun onMain(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) block() else main.post { block() }
    }

    private fun withPlayer(block: (ExoPlayer) -> Unit) {
        onMain {
            val p = DragonAudioService.player
            if (p != null && pending.isEmpty()) {
                block(p)
            } else {
                pending.add(block)
                if (!connecting) {
                connecting = true
                val token = SessionToken(ctx, ComponentName(ctx, DragonAudioService::class.java))
                val fut = MediaController.Builder(ctx, token).buildAsync()
                fut.addListener({
                    connecting = false
                    try { controller = fut.get() } catch (e: Exception) { lastError = e.toString() }
                    val pl = DragonAudioService.player
                    val todo = ArrayList(pending)
                    pending.clear()
                    if (pl != null) todo.forEach { it(pl) }
                }, ContextCompat.getMainExecutor(ctx))
                }
            }
        }
    }

    private fun item(o: JSONObject): MediaItem {
        val meta = MediaMetadata.Builder()
            .setTitle(o.optString("title"))
            .setArtist(o.optString("artist"))
            .setAlbumTitle(o.optString("album"))
        val aid = o.optString("albumId")
        if (aid.isNotEmpty()) {
            val f = artFile(aid.toLongOrNull() ?: 0L, 512)
            meta.setArtworkUri(Uri.fromFile(f))
        }
        return MediaItem.Builder()
            .setMediaId(o.optString("id"))
            .setUri(o.optString("uri"))
            .setMediaMetadata(meta.build())
            .build()
    }

    private fun repeatName(m: Int) = when (m) { Player.REPEAT_MODE_ONE -> "one"; Player.REPEAT_MODE_ALL -> "all"; else -> "off" }

    fun handle(method: String, a: JSONObject, reply: (Any?) -> Unit): Boolean {
        when (method) {
            "audio.state" -> {
                onMain { reply(state()) }
            }
            "audio.setQueue" -> {
                val arr = a.optJSONArray("items") ?: JSONArray()
                val start = a.optInt("index", 0)
                val play = a.optBoolean("play", true)
                onMain {
                    arr.optJSONObject(start)?.let { art(it.optString("albumId").toLongOrNull() ?: 0L, 512) }
                    val items = ArrayList<MediaItem>()
                    for (i in 0 until arr.length()) items.add(item(arr.getJSONObject(i)))
                    withPlayer { p ->
                        p.setMediaItems(items, start.coerceIn(0, (items.size - 1).coerceAtLeast(0)), 0L)
                        p.prepare()
                        if (play) p.play()
                        reply(true)
                    }
                    io.execute {
                        val seen = HashSet<String>()
                        for (i in 0 until arr.length()) {
                            val id = arr.getJSONObject(i).optString("albumId")
                            if (id.isNotEmpty() && seen.add(id)) art(id.toLongOrNull() ?: 0L, 512)
                        }
                    }
                }
            }
            "audio.play" -> withPlayer { it.play() }
            "audio.pause" -> withPlayer { it.pause() }
            "audio.toggle" -> withPlayer { if (it.isPlaying) it.pause() else { if (it.playbackState == Player.STATE_ENDED) it.seekTo(0, 0L); it.play() } }
            "audio.next" -> withPlayer { it.seekToNextMediaItem() }
            "audio.previous" -> withPlayer { if (it.currentPosition > 3000) it.seekTo(0) else it.seekToPreviousMediaItem() }
            "audio.seek" -> withPlayer { it.seekTo(a.optLong("ms", 0L)) }
            "audio.skipTo" -> withPlayer { it.seekTo(a.optInt("index", 0), 0L); it.play() }
            "audio.shuffle" -> withPlayer { it.shuffleModeEnabled = a.optBoolean("on") }
            "audio.repeat" -> withPlayer {
                it.repeatMode = when (a.optString("mode")) { "one" -> Player.REPEAT_MODE_ONE; "all" -> Player.REPEAT_MODE_ALL; else -> Player.REPEAT_MODE_OFF }
            }
            "audio.speed" -> withPlayer { it.playbackParameters = PlaybackParameters(a.optDouble("value", 1.0).toFloat()) }
            "audio.stop" -> withPlayer { it.stop(); it.clearMediaItems(); setSleep(0) }
            "audio.queue.add" -> {
                val o = a.optJSONObject("item")
                if (o != null) onMain {
                    art(o.optString("albumId").toLongOrNull() ?: 0L, 512)
                    withPlayer { p ->
                        val mi = item(o)
                        if (p.mediaItemCount == 0) { p.setMediaItem(mi); p.prepare() }
                        else if (a.optBoolean("next")) p.addMediaItem(p.currentMediaItemIndex + 1, mi)
                        else p.addMediaItem(mi)
                    }
                }
            }
            "audio.queue.remove" -> withPlayer { val i = a.optInt("index", -1); if (i in 0 until it.mediaItemCount) it.removeMediaItem(i) }
            "audio.queue.move" -> withPlayer {
                val f = a.optInt("from", -1); val t = a.optInt("to", -1)
                if (f in 0 until it.mediaItemCount && t in 0 until it.mediaItemCount) it.moveMediaItem(f, t)
            }
            "audio.sleep" -> onMain { setSleep(a.optLong("minutes", 0L) * 60_000L); reply(true) }
            "audio.eq.get" -> withPlayer { reply(eqInfo(it)) }
            "audio.eq.set" -> withPlayer { p -> applyEq(p, a); reply(eqInfo(p)) }
            else -> return false
        }
        return true
    }

    private fun state(): JSONObject {
        val o = JSONObject()
        val p = DragonAudioService.player
        if (p == null) {
            return o.put("active", false).put("playing", false).put("positionMs", 0).put("durationMs", 0)
                .put("index", -1).put("count", 0).put("shuffle", false).put("repeat", "off").put("speed", 1.0)
                .put("sleepLeftMs", 0)
        }
        val dur = p.duration
        o.put("active", p.mediaItemCount > 0)
        o.put("playing", p.isPlaying)
        o.put("buffering", p.playbackState == Player.STATE_BUFFERING)
        o.put("ended", p.playbackState == Player.STATE_ENDED)
        o.put("positionMs", p.currentPosition)
        o.put("durationMs", if (dur == androidx.media3.common.C.TIME_UNSET) 0L else dur)
        o.put("index", if (p.mediaItemCount == 0) -1 else p.currentMediaItemIndex)
        o.put("count", p.mediaItemCount)
        o.put("mediaId", p.currentMediaItem?.mediaId ?: "")
        o.put("nextIndex", p.nextMediaItemIndex)
        o.put("shuffle", p.shuffleModeEnabled)
        o.put("repeat", repeatName(p.repeatMode))
        o.put("speed", p.playbackParameters.speed.toDouble())
        o.put("sleepLeftMs", if (sleepAt > 0) (sleepAt - System.currentTimeMillis()).coerceAtLeast(0) else 0L)
        if (p.playerError != null) o.put("error", p.playerError.toString())
        return o
    }


    private fun setSleep(ms: Long) {
        main.removeCallbacks(sleepRunnable)
        if (ms <= 0) { sleepAt = 0; return }
        sleepAt = System.currentTimeMillis() + ms
        main.postDelayed(sleepRunnable, ms)
    }

    private fun fadeOutAndPause() {
        val p = DragonAudioService.player
        sleepAt = 0
        if (p == null) return
        val steps = 20
        var i = 0
        val r = object : Runnable {
            override fun run() {
                i++
                p.volume = (1f - i.toFloat() / steps).coerceAtLeast(0f)
                if (i < steps) main.postDelayed(this, 150) else { p.pause(); p.volume = 1f }
            }
        }
        main.post(r)
    }


    private fun ensureFx(p: ExoPlayer) {
        val sid = p.audioSessionId
        if (sid == 0 || sid == eqSession) return
        try { eq?.release(); bass?.release() } catch (_: Exception) {}
        eq = try { Equalizer(0, sid) } catch (e: Exception) { null }
        bass = try { BassBoost(0, sid) } catch (e: Exception) { null }
        eqSession = sid
    }

    private fun eqInfo(p: ExoPlayer): JSONObject {
        ensureFx(p)
        val e = eq ?: return JSONObject().put("available", false)
        val o = JSONObject().put("available", true).put("enabled", e.enabled)
        val range = e.bandLevelRange
        o.put("minLevel", range[0].toInt()).put("maxLevel", range[1].toInt())
        val bands = JSONArray()
        for (b in 0 until e.numberOfBands) {
            bands.put(JSONObject().put("hz", e.getCenterFreq(b.toShort()) / 1000).put("level", e.getBandLevel(b.toShort()).toInt()))
        }
        o.put("bands", bands)
        val presets = JSONArray()
        for (i in 0 until e.numberOfPresets) presets.put(e.getPresetName(i.toShort()))
        o.put("presets", presets)
        o.put("bass", try { bass?.roundedStrength?.toInt() ?: 0 } catch (_: Exception) { 0 })
        o.put("bassSupported", bass?.strengthSupported == true)
        return o
    }

    private fun applyEq(p: ExoPlayer, a: JSONObject) {
        ensureFx(p)
        val e = eq ?: return
        if (a.has("enabled")) e.enabled = a.optBoolean("enabled")
        if (a.has("preset")) { e.usePreset(a.optInt("preset").toShort()) }
        if (a.has("band")) e.setBandLevel(a.optInt("band").toShort(), a.optInt("level").toShort())
        if (a.has("levels")) {
            val l = a.optJSONArray("levels")
            if (l != null) for (i in 0 until minOf(l.length(), e.numberOfBands.toInt())) e.setBandLevel(i.toShort(), l.optInt(i).toShort())
        }
        if (a.has("bass")) {
            try { bass?.setStrength(a.optInt("bass").toShort()); bass?.enabled = a.optInt("bass") > 0 } catch (_: Exception) {}
        }
    }
}
