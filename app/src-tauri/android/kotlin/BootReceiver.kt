package ai.spectynmesh.app

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * Revives the Spectyn Mesh node after a device reboot.
 *
 * On reboot the OS tears down [MeshNodeService] (the persistent specialUse
 * foreground service) and any in-flight WorkManager jobs need re-evaluation.
 * This receiver does best-effort revival when the boot broadcasts fire:
 *   - re-schedule the periodic upkeep worker (idempotent), and
 *   - (re)start the mesh node foreground service.
 *
 * Registered in AndroidManifest by android/inject.sh with an intent-filter for
 * BOOT_COMPLETED (+ LOCKED_BOOT_COMPLETED) and the RECEIVE_BOOT_COMPLETED perm.
 *
 * NOTE: this handles the REBOOT case only. Starting an FGS from a *background*
 * (non-boot) context is blocked on Android 12+ (API 31+) and truly needs a
 * high-priority FCM data push — that remains a separate follow-up and is NOT
 * solved here. Boot-time FGS start is generally permitted for a registered FGS,
 * but we still wrap it in try/catch and swallow if the OS restricts it.
 */
class BootReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent?) {
        val action = intent?.action ?: return
        if (action != Intent.ACTION_BOOT_COMPLETED &&
            action != "android.intent.action.LOCKED_BOOT_COMPLETED" &&
            action != "android.intent.action.QUICKBOOT_POWERON" &&
            action != "com.htc.intent.action.QUICKBOOT_POWERON"
        ) {
            return
        }

        // Re-schedule periodic upkeep (idempotent — KEEP existing).
        FocusUpkeepWorker.schedule(context)

        // Best-effort start the mesh node. On boot this is generally permitted
        // for the registered specialUse FGS; swallow if the OS restricts it.
        // (The API31+ *background* non-boot start limitation is a separate FCM
        // follow-up — not solved here.)
        try {
            context.startForegroundService(
                Intent(context, MeshNodeService::class.java)
            )
        } catch (_: Exception) {
            // Boot-time FGS start restricted on this OEM/OS — node will instead
            // come back on next app launch / START_STICKY restart. Non-fatal.
        }
    }
}
