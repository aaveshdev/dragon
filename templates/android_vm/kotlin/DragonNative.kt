package org.dragonlang.runtime

object DragonNative {
    init {
        System.loadLibrary("dragon_lib")
    }

    external fun init(appDir: String, docsDir: String, mediaJson: String): String

    external fun render(): String

    external fun event(evt: String): Boolean
    external fun isDirty(): Boolean

    external fun tick(nowMs: Long): Long

    external fun platformCalls(): String
    external fun platformResult(id: String, resultJson: String): Boolean

    external fun back(): Boolean
    external fun setMedia(mediaJson: String): Boolean
    external fun lifecycle(state: String): Boolean
    external fun lastError(): String
}
