package ai.phantommesh.app

import android.annotation.SuppressLint
import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

/**
 * SPEC-12 §7.3 — Android identity keystore JNI bridge (Kotlin counterpart).
 *
 * This object is the Kotlin side of the `core/src/identity_wire.rs` Android
 * keystore arm. The Rust bridge resolves this exact class over JNI:
 *
 *   find_class("ai/phantommesh/app/IdentityKeystore")
 *
 * and invokes these three STATIC methods (must stay `@JvmStatic` so the Rust
 * `call_static_method` finds them) with these EXACT signatures — keep them in
 * lock-step with the `ANDROID_SIG_*` / `ANDROID_IDENTITY_*_METHOD` constants in
 * identity_wire.rs:
 *
 *   write(account: String, b64Value: String): Unit   // (Ljava/lang/String;Ljava/lang/String;)V
 *   read(account: String): String?                    // (Ljava/lang/String;)Ljava/lang/String;
 *   delete(account: String): Unit                     // (Ljava/lang/String;)V
 *
 * Storage: an [EncryptedSharedPreferences] file (`phantom_identity_keystore`)
 * whose AES-256-GCM data/key encryption keys are wrapped by a [MasterKey] held
 * in the hardware-backed AndroidKeyStore (`AES256_GCM` scheme). The Rust side
 * base64-encodes the raw 32-byte master seed BEFORE calling `write`, so the
 * value crossing JNI — and the value we persist — is always the printable b64
 * string; `read` returns that same string verbatim (or `null` when absent, which
 * the Rust bridge maps to `KeyDerivationError::MasterNotFound`).
 *
 * Context: these methods are called from native code with no Activity/Context
 * handle, so we recover the process-wide application [Context] reflectively via
 * `ActivityThread.currentApplication()` — the standard pattern for a Context-less
 * static helper invoked over JNI. The lookup is cached after first success.
 *
 * 中文: 這是 `identity_wire.rs` Android keystore arm 的 Kotlin 對應實作。Rust 透過
 * JNI `find_class` + `call_static_method` 呼叫底下三個 `@JvmStatic` 方法（簽章必須
 * 與 Rust 端 `ANDROID_SIG_*` 常數完全一致）。master seed 由 Rust 端先 base64 編碼，
 * 我們把該 b64 字串存進由 AndroidKeyStore 硬體金鑰包覆的
 * EncryptedSharedPreferences；`read` 原樣回傳該字串，不存在時回 `null`
 * （Rust 端會對應成 `MasterNotFound`）。
 */
object IdentityKeystore {

    /** EncryptedSharedPreferences file name backing every identity record. */
    private const val PREFS_FILE = "phantom_identity_keystore"

    /** Cached encrypted prefs handle — built lazily on first use, then reused. */
    @Volatile
    private var cachedPrefs: SharedPreferences? = null

    /**
     * Persist `b64Value` (the base64-encoded master seed produced by Rust) under
     * the `account` key. Upsert: re-`init` overwrites the prior value in place,
     * mirroring the macOS/Linux backend idempotent-write contract.
     */
    @JvmStatic
    fun write(account: String, b64Value: String) {
        // commit() (synchronous), NOT apply(): the Rust bridge deletes the legacy
        // plaintext seed immediately after write() returns, so the encrypted value
        // MUST be durably on disk before we hand control back over JNI. apply()'s
        // async background flush could lose the only copy of the master seed on an
        // early process death between the write and the legacy-file removal.
        if (!prefs().edit().putString(account, b64Value).commit()) {
            throw IllegalStateException(
                "IdentityKeystore.write: EncryptedSharedPreferences commit() failed for account=$account"
            )
        }
    }

    /**
     * Return the base64 string previously stored for `account`, or `null` when
     * no record exists. The Rust bridge treats a JNI-null return as
     * `KeyDerivationError::MasterNotFound`.
     */
    @JvmStatic
    fun read(account: String): String? {
        return prefs().getString(account, null)
    }

    /**
     * Remove `account`'s record. Idempotent — removing an absent key is a no-op
     * success, matching the file / Linux / macOS delete arms.
     */
    @JvmStatic
    fun delete(account: String) {
        // commit() (synchronous) for the same durability guarantee as write():
        // the caller may rely on the removal being on disk before proceeding.
        if (!prefs().edit().remove(account).commit()) {
            throw IllegalStateException(
                "IdentityKeystore.delete: EncryptedSharedPreferences commit() failed for account=$account"
            )
        }
    }

    /**
     * Lazily build (and cache) the AndroidKeyStore-backed
     * EncryptedSharedPreferences handle. The [MasterKey] uses the `AES256_GCM`
     * scheme so its underlying key material lives in the hardware-backed
     * AndroidKeyStore; EncryptedSharedPreferences then AES-256-GCM-encrypts both
     * entry keys and values at rest.
     */
    private fun prefs(): SharedPreferences {
        cachedPrefs?.let { return it }
        synchronized(this) {
            cachedPrefs?.let { return it }
            val ctx = appContext().applicationContext
            val masterKey = MasterKey.Builder(ctx)
                .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                .build()
            val built = EncryptedSharedPreferences.create(
                ctx,
                PREFS_FILE,
                masterKey,
                EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
                EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
            )
            cachedPrefs = built
            return built
        }
    }

    /**
     * Recover the process application [Context] reflectively. These methods are
     * invoked from native code that holds no Context, so we read the running
     * app instance from `ActivityThread.currentApplication()` (public-by-effect
     * across all supported API levels). Throws [IllegalStateException] if the
     * application object is not yet available, which surfaces back through JNI as
     * a Java exception the Rust bridge converts into `KeystoreUnavailable`.
     */
    @SuppressLint("PrivateApi")
    private fun appContext(): Context {
        val activityThread = Class.forName("android.app.ActivityThread")
        val currentApplication = activityThread.getMethod("currentApplication")
        val app = currentApplication.invoke(null) as? Context
            ?: throw IllegalStateException(
                "IdentityKeystore: application Context unavailable " +
                    "(ActivityThread.currentApplication() returned null)"
            )
        return app
    }
}
