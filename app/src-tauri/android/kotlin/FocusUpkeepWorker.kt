package ai.spectynmesh.app

import android.app.NotificationManager
import android.content.ComponentName
import android.content.Context
import android.service.quicksettings.TileService
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import java.util.concurrent.TimeUnit

/**
 * SPEC-34 periodic upkeep worker (WorkManager), ~15 min cadence.
 *
 * **Focus-session cleanup**: if a focus session's end time has passed but its
 * state was never cleared (process killed before STOP_FOCUS ran), clear it.
 * Critically this is done WITHOUT starting a foreground service: a periodic
 * WorkManager job runs in the background, and `startForegroundService()` from a
 * background context throws `ForegroundServiceStartNotAllowedException` on
 * Android 12+ (API 31+). So the worker clears [FocusSessionState] and cancels
 * the focus notification directly via [NotificationManager] (no FGS needed),
 * then nudges the Quick Settings tile to repaint.
 *
 * **Mesh keepalive — deliberately NOT done here.** The previous implementation
 * called `startForegroundService(MeshNodeService)` from `doWork()`, which is
 * silently blocked on API 31+ (the exception was swallowed → dead code that
 * looked like keepalive but did nothing). Node revival instead relies on
 * `MeshNodeService`'s `START_STICKY` (the OS restarts it after a kill). True
 * background revival (high-priority FCM data push, or a user-granted exact
 * alarm) is a follow-up — see SPEC-34 / SPEC-33 background-survival notes.
 *
 * Always returns [Result.success] — best-effort upkeep, never a retry-storm.
 */
class FocusUpkeepWorker(
    appContext: Context,
    params: WorkerParameters,
) : CoroutineWorker(appContext, params) {

    override suspend fun doWork(): Result {
        val ctx = applicationContext
        if (FocusSessionState.hasStaleSession(ctx)) {
            FocusSessionState.stop(ctx)
            // Cancel the lingering focus notification directly (no service start).
            ctx.getSystemService(NotificationManager::class.java)
                ?.cancel(MeshNodeService.FOCUS_NOTIFICATION_ID)
            requestTileRefresh(ctx)
        }
        return Result.success()
    }

    /** Repaint the Quick Settings tile (does not require a foreground service). */
    private fun requestTileRefresh(ctx: Context) {
        try {
            TileService.requestListeningState(
                ctx,
                ComponentName(ctx, FocusQuickTile::class.java),
            )
        } catch (_: Exception) {
            // Tile not added / unavailable — non-fatal.
        }
    }

    companion object {
        private const val UNIQUE_WORK = "spectyn_focus_upkeep"
        private const val INTERVAL_MINUTES = 15L

        /** Enqueue the periodic upkeep worker (idempotent — KEEP existing). */
        fun schedule(context: Context) {
            val request = PeriodicWorkRequestBuilder<FocusUpkeepWorker>(
                INTERVAL_MINUTES, TimeUnit.MINUTES,
            ).build()
            WorkManager.getInstance(context).enqueueUniquePeriodicWork(
                UNIQUE_WORK,
                ExistingPeriodicWorkPolicy.KEEP,
                request,
            )
        }
    }
}
