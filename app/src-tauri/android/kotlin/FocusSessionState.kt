package ai.phantommesh.app

import android.content.Context

/**
 * Lightweight persistent state for the current SPEC-21 focus session.
 *
 * Backed by SharedPreferences so the Quick Settings tile (FocusQuickTile)
 * and the foreground service (MeshNodeService) agree on whether a session is
 * running and when it ends — even across process restarts.
 *
 * SPEC-34 §10E. Kept deliberately tiny (no DataStore/coroutines) so it's
 * safe to read synchronously from TileService.onStartListening().
 */
object FocusSessionState {

    private const val PREFS = "phantom_focus_session"
    private const val KEY_END_AT_MS = "focus_end_at_ms"
    private const val KEY_STARTED_AT_MS = "focus_started_at_ms"

    private fun prefs(ctx: Context) =
        ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** Mark a focus session active, ending [minutes] from now. */
    fun start(ctx: Context, minutes: Int) {
        val now = System.currentTimeMillis()
        prefs(ctx).edit()
            .putLong(KEY_STARTED_AT_MS, now)
            .putLong(KEY_END_AT_MS, now + minutes * 60_000L)
            .apply()
    }

    /** Clear the active session (user stopped, or it finished). */
    fun stop(ctx: Context) {
        prefs(ctx).edit()
            .remove(KEY_END_AT_MS)
            .remove(KEY_STARTED_AT_MS)
            .apply()
    }

    /** True if a session is running and hasn't elapsed yet. */
    fun isActive(ctx: Context): Boolean {
        val endAt = prefs(ctx).getLong(KEY_END_AT_MS, 0L)
        return endAt > System.currentTimeMillis()
    }

    /** Whole minutes remaining (0 if no active session). */
    fun remainingMinutes(ctx: Context): Int {
        val endAt = prefs(ctx).getLong(KEY_END_AT_MS, 0L)
        val remainingMs = endAt - System.currentTimeMillis()
        return if (remainingMs <= 0) 0 else ((remainingMs + 59_999L) / 60_000L).toInt()
    }

    /** Whole minutes elapsed since the session started (0 if none). */
    fun elapsedMinutes(ctx: Context): Int {
        val startedAt = prefs(ctx).getLong(KEY_STARTED_AT_MS, 0L)
        if (startedAt <= 0L) return 0
        val elapsedMs = System.currentTimeMillis() - startedAt
        return if (elapsedMs <= 0) 0 else (elapsedMs / 60_000L).toInt()
    }

    /**
     * True if a session's end time has passed but its state was never cleared
     * (e.g. the process was killed before [stop] ran). This is the cleanup
     * target for the periodic [FocusUpkeepWorker] — distinct from [isActive],
     * which is false for both "no session" and "finished-but-stale".
     */
    fun hasStaleSession(ctx: Context): Boolean {
        val endAt = prefs(ctx).getLong(KEY_END_AT_MS, 0L)
        return endAt in 1 until System.currentTimeMillis() + 1
    }
}
