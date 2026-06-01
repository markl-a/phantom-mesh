package ai.phantommesh.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.IBinder
import android.service.quicksettings.TileService
import androidx.core.app.NotificationCompat

/**
 * Foreground service that keeps the Phantom Mesh HTTP node alive
 * even when the app is in the background or the user swipes it away.
 *
 * Lifecycle: started by MainActivity.onCreate(), runs until the user
 * explicitly stops it from the notification or kills the process.
 *
 * Also hosts the SPEC-21 focus session (started by FocusQuickTile or the
 * in-app capture surface): ACTION_START_FOCUS / ACTION_STOP_FOCUS.
 */
class MeshNodeService : Service() {

    companion object {
        const val CHANNEL_ID   = "phantom_mesh_node"
        const val NOTIFICATION_ID = 1001
        const val ACTION_STOP  = "ai.phantommesh.app.STOP_NODE"

        // SPEC-21 focus session (mirrors FocusQuickTile constants)
        const val ACTION_START_FOCUS  = "ai.phantommesh.app.START_FOCUS"
        const val ACTION_STOP_FOCUS   = "ai.phantommesh.app.STOP_FOCUS"
        const val EXTRA_FOCUS_MINUTES = "focus_minutes"
        const val FOCUS_CHANNEL_ID    = "phantom_focus_session"
        const val FOCUS_NOTIFICATION_ID = 1002
        const val DEFAULT_FOCUS_MINUTES = 25

        // SPEC-34 §10E habit-chip palette widget (mirrors HabitChipPaletteWidget)
        const val ACTION_CAPTURE_HABIT = "ai.phantommesh.app.CAPTURE_HABIT"
        const val EXTRA_HABIT_SLUG     = "habit_slug"
        const val HABIT_NOTIFICATION_ID = 1003
    }

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
        createFocusChannel()
        // SPEC-34: enqueue periodic upkeep (stale-focus cleanup). Idempotent.
        FocusUpkeepWorker.schedule(this)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
                return START_NOT_STICKY
            }
            ACTION_START_FOCUS -> {
                val minutes = intent.getIntExtra(EXTRA_FOCUS_MINUTES, DEFAULT_FOCUS_MINUTES)
                startFocusSession(minutes)
                // Keep the node foreground notification alive too
                startForeground(NOTIFICATION_ID, buildNotification())
                return START_STICKY
            }
            ACTION_STOP_FOCUS -> {
                stopFocusSession()
                startForeground(NOTIFICATION_ID, buildNotification())
                return START_STICKY
            }
            ACTION_CAPTURE_HABIT -> {
                val slug = intent.getStringExtra(EXTRA_HABIT_SLUG) ?: "log"
                captureHabit(slug)
                // Keep the node foreground notification alive (we may have been
                // started fresh by the widget tap).
                startForeground(NOTIFICATION_ID, buildNotification())
                return START_STICKY
            }
        }

        startForeground(NOTIFICATION_ID, buildNotification())
        // START_STICKY: Android restarts the service if it's killed
        return START_STICKY
    }

    /** Begin a SPEC-21 focus session: persist state, post countdown notification, refresh tile. */
    private fun startFocusSession(minutes: Int) {
        FocusSessionState.start(this, minutes)
        getSystemService(NotificationManager::class.java)
            .notify(FOCUS_NOTIFICATION_ID, buildFocusNotification(minutes))
        requestTileRefresh()
    }

    /** End the focus session: clear state, cancel notification, refresh tile. */
    private fun stopFocusSession() {
        FocusSessionState.stop(this)
        getSystemService(NotificationManager::class.java)
            .cancel(FOCUS_NOTIFICATION_ID)
        requestTileRefresh()
    }

    /**
     * Record a one-tap habit capture from the SPEC-34 §10E palette widget.
     *
     * The capture is appended to a tiny SharedPreferences-backed pending queue
     * (newline-delimited "<epochMs>,<slug>") that the in-app WebView / core node
     * drains into the real capture-habit pipeline on next foreground. We persist
     * here rather than calling the pipeline directly because the widget can start
     * this service from a cold process where the JS bridge isn't up yet.
     * A short auto-cancel notification confirms the tap to the user.
     */
    private fun captureHabit(slug: String) {
        // F1 hardening: validate the slug against the fixed SPEC-34 §10E widget
        // chip allowlist BEFORE persisting/displaying. A malformed/unexpected
        // intent extra must not inject arbitrary content into the queue or
        // notification; anything off-list coerces to the safe default "water".
        // These slugs MUST match HabitChipPaletteWidget + STARTER_PALETTE
        // (app/src/lib/captureHabit.ts) so drained taps map to real habit chips.
        val validSlug = if (slug in setOf("water", "coffee", "exercise", "breath")) slug else "water"

        val prefs = getSharedPreferences("phantom_habit_queue", Context.MODE_PRIVATE)
        val pending = prefs.getString("pending", "").orEmpty()
        val entry = "${System.currentTimeMillis()},$validSlug"
        prefs.edit().putString("pending", if (pending.isEmpty()) entry else "$pending\n$entry").apply()

        val note = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("已記錄 · $validSlug")
            .setContentText("習慣已加入待同步佇列")
            .setSmallIcon(R.mipmap.ic_launcher)
            .setAutoCancel(true)
            .setTimeoutAfter(4_000L)
            .build()
        getSystemService(NotificationManager::class.java)
            .notify(HABIT_NOTIFICATION_ID, note)
    }

    /** Ask the Quick Settings tile to re-read FocusSessionState + repaint. */
    private fun requestTileRefresh() {
        try {
            TileService.requestListeningState(
                this,
                ComponentName(this, FocusQuickTile::class.java)
            )
        } catch (_: Exception) {
            // Tile not added by user / not available — non-fatal
        }
    }

    private fun buildFocusNotification(minutes: Int): Notification {
        val stopFocus = PendingIntent.getService(
            this, 2,
            Intent(this, MeshNodeService::class.java).apply { action = ACTION_STOP_FOCUS },
            PendingIntent.FLAG_IMMUTABLE
        )
        return NotificationCompat.Builder(this, FOCUS_CHANNEL_ID)
            .setContentTitle("專注中 · Phantom 焦點")
            .setContentText("$minutes 分鐘焦點 session 進行中")
            .setSmallIcon(R.drawable.ic_focus_tile)
            .setOngoing(true)
            .setUsesChronometer(true)
            .addAction(0, "結束", stopFocus)
            .build()
    }

    private fun createFocusChannel() {
        val channel = NotificationChannel(
            FOCUS_CHANNEL_ID,
            "Phantom 焦點 session",
            NotificationManager.IMPORTANCE_LOW
        ).apply {
            description = "SPEC-21 焦點 session 進行中的通知"
            setShowBadge(false)
        }
        getSystemService(NotificationManager::class.java)
            .createNotificationChannel(channel)
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onTaskRemoved(rootIntent: Intent?) {
        // App swiped away — keep the service running (node stays online)
        super.onTaskRemoved(rootIntent)
    }

    private fun buildNotification(): Notification {
        val openIntent = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE
        )
        val stopIntent = PendingIntent.getService(
            this, 1,
            Intent(this, MeshNodeService::class.java).apply { action = ACTION_STOP },
            PendingIntent.FLAG_IMMUTABLE
        )

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Phantom Mesh")
            .setContentText("節點運行中 · port 7878")
            .setSmallIcon(R.mipmap.ic_launcher)
            .setOngoing(true)
            .setContentIntent(openIntent)
            .addAction(0, "停止節點", stopIntent)
            .build()
    }

    private fun createNotificationChannel() {
        val channel = NotificationChannel(
            CHANNEL_ID,
            "Phantom Mesh 節點",
            NotificationManager.IMPORTANCE_LOW
        ).apply {
            description = "保持 Phantom Mesh 節點在背景運行"
            setShowBadge(false)
        }
        getSystemService(NotificationManager::class.java)
            .createNotificationChannel(channel)
    }
}
