package org.dragonlang.runtime

import org.json.JSONObject

class DragonAudioPlugin(ctx: PluginContext) : DragonPlugin {
    override val namespace = "audio"
    private val audio = DragonAudio(ctx.appContext, ctx.main)

    override fun handles(method: String) = method.startsWith("audio.") || method == "media.query" || method == "media.art"

    override fun handle(method: String, args: JSONObject, reply: (Any?) -> Unit): Boolean = when (method) {
        "media.query" -> { audio.query(reply, args.optLong("minDurationMs", 30000L)); true }
        "media.art" -> { audio.artAsync(args.optString("albumId").toLongOrNull() ?: 0L, args.optInt("size", 256), reply); true }
        else -> audio.handle(method, args, reply)
    }
}
