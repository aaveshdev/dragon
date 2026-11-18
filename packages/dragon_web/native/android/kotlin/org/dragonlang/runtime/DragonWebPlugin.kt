package org.dragonlang.runtime

import android.content.Context
import android.view.View
import org.json.JSONObject

class DragonWebPlugin(ctx: PluginContext) : DragonPlugin {
    override val namespace = "web"
    private val web = DragonWeb(ctx.activity, ctx.main, ctx.docsDir)

    override fun handle(method: String, args: JSONObject, reply: (Any?) -> Unit): Boolean = web.handle(method, args, reply)

    override fun createView(kind: String, id: String, params: JSONObject, context: Context): View? =
        if (kind == "web") (web.viewFor(params.optString("tab", id)) ?: View(context)) else null

    override fun releaseView(kind: String, id: String) { if (kind == "web") web.released(id) }
}
