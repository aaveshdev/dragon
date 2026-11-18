package org.dragonlang.runtime

import android.content.Context
import android.os.Handler
import android.view.View
import androidx.activity.ComponentActivity
import org.json.JSONObject
import java.io.File

class PluginContext(
    val activity: ComponentActivity,
    val main: Handler,
    val docsDir: File,
) {
    val appContext: Context get() = activity.applicationContext
}

interface DragonPlugin {
    val namespace: String

    fun handles(method: String): Boolean = method.startsWith("$namespace.")

    fun handle(method: String, args: JSONObject, reply: (Any?) -> Unit): Boolean = false

    fun createView(kind: String, id: String, params: JSONObject, context: Context): View? = null

    fun releaseView(kind: String, id: String) {}
}
