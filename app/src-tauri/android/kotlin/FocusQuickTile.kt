package ai.phantommesh.app

import android.content.Intent
import android.graphics.drawable.Icon
import android.os.Handler
import android.os.Looper
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService

/**
 * Quick Settings tile — "Phantom 焦點" / "Phantom Focus".
 *
 * SPEC-34 §10E / SPEC-21 capture-focus.
 *
 * One tap from the notification shade starts a 25-minute SPEC-21 focus
 * session without unlocking → app icon → cold start → find button (the old
 * 6-step path). The tile reflects active state with a countdown subtitle.
 *
 * User must add the tile manually once (Android 13+ has no API to auto-add);
 * the first-run wizard guides them (SPEC-34 §6 "tile add" coach mark).
 *
 * Wiring: tap → ACTION_START_FOCUS broadcast → MeshNodeService starts the
 * focus session foreground task → service broadcasts focus state back so the
 * tile can flip to active + show remaining minutes.
 */
class FocusQuickTile : TileService() {

    companion object {
        const val ACTION_START_FOCUS = "ai.phantommesh.app.START_FOCUS"
        const val ACTION_STOP_FOCUS  = "ai.phantommesh.app.STOP_FOCUS"
        const val EXTRA_FOCUS_MINUTES = "focus_minutes"
        const val DEFAULT_FOCUS_MINUTES = 25
        private const val REFRESH_INTERVAL_MS = 30_000L
    }

    private val refreshHandler = Handler(Looper.getMainLooper())
    private val countdownTick = object : Runnable {
        override fun run() {
            // Re-read state and repaint while the shade is open so the active
            // tile shows a live-decreasing "剩 N 分" subtitle (and flips to idle
            // the moment the session elapses). Reschedules every 30s.
            refreshTileState()
            if (FocusSessionState.isActive(this@FocusQuickTile)) {
                refreshHandler.postDelayed(this, REFRESH_INTERVAL_MS)
            }
        }
    }

    /** Tile becomes visible in the shade — sync its state + start live countdown. */
    override fun onStartListening() {
        super.onStartListening()
        refreshHandler.removeCallbacks(countdownTick)
        refreshTileState()
        if (FocusSessionState.isActive(this)) {
            refreshHandler.postDelayed(countdownTick, REFRESH_INTERVAL_MS)
        }
    }

    /** Shade closed — stop the countdown loop so we don't tick in the background. */
    override fun onStopListening() {
        refreshHandler.removeCallbacks(countdownTick)
        super.onStopListening()
    }

    /** User tapped the tile. */
    override fun onClick() {
        super.onClick()
        val tile = qsTile ?: return

        when (tile.state) {
            Tile.STATE_ACTIVE -> {
                // SPEC-34 §17-F anti-misfire: a stray shade tap must NOT end a
                // running session. Open the app to the focus screen instead; the
                // session ends only from in-app 結束 or the notification action.
                openFocusInApp()
                // keep the tile ACTIVE — do not flip to inactive
            }
            else -> {
                // Idle → start a 25-min focus session
                sendFocusIntent(ACTION_START_FOCUS, DEFAULT_FOCUS_MINUTES)
                setTileActive(DEFAULT_FOCUS_MINUTES)
            }
        }
    }

    /**
     * SPEC-34 §17-F: deep-link the app to the focus screen from an active tile.
     * Uses startActivityAndCollapse so the shade closes; on Android 14+ this
     * requires a PendingIntent overload. The session keeps running — we never
     * send ACTION_STOP_FOCUS here.
     */
    private fun openFocusInApp() {
        val launch = Intent(this, MainActivity::class.java).apply {
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            putExtra("focus_route", "/focus")
        }
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            val pi = android.app.PendingIntent.getActivity(
                this, 0, launch,
                android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_UPDATE_CURRENT,
            )
            startActivityAndCollapse(pi)
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(launch)
        }
    }

    private fun sendFocusIntent(action: String, minutes: Int = 0) {
        val intent = Intent(this, MeshNodeService::class.java).apply {
            this.action = action
            if (minutes > 0) putExtra(EXTRA_FOCUS_MINUTES, minutes)
        }
        // Foreground service start is allowed from a TileService click
        // (user-initiated, exempt from background-start restrictions).
        startForegroundService(intent)
    }

    private fun setTileActive(minutes: Int) {
        qsTile?.apply {
            state = Tile.STATE_ACTIVE
            label = getString(R.string.focus_tile_label)
            subtitle = getString(R.string.focus_tile_active_subtitle, minutes)
            icon = Icon.createWithResource(this@FocusQuickTile, R.drawable.ic_focus_tile)
            updateTile()
        }
    }

    private fun setTileInactive() {
        qsTile?.apply {
            state = Tile.STATE_INACTIVE
            label = getString(R.string.focus_tile_label)
            subtitle = getString(R.string.focus_tile_idle_subtitle)
            icon = Icon.createWithResource(this@FocusQuickTile, R.drawable.ic_focus_tile)
            updateTile()
        }
    }

    /**
     * Reconcile tile visual with the actual focus-session state.
     *
     * MVP: defaults to inactive. A follow-up wires MeshNodeService to persist
     * the active-session end-time (DataStore) so the tile can show the live
     * countdown after a process restart. Tracked in SPEC-34 §10E follow-up.
     */
    private fun refreshTileState() {
        val active = FocusSessionState.isActive(this)
        if (active) {
            setTileActive(FocusSessionState.remainingMinutes(this))
        } else {
            setTileInactive()
        }
    }
}
