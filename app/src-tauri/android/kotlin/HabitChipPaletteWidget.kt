package ai.phantommesh.app

import android.content.Context
import android.content.Intent
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.GlanceTheme
import androidx.glance.action.ActionParameters
import androidx.glance.action.actionParametersOf
import androidx.glance.action.clickable
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.action.ActionCallback
import androidx.glance.appwidget.action.actionRunCallback
import androidx.glance.appwidget.cornerRadius
import androidx.glance.appwidget.provideContent
import androidx.glance.background
import androidx.glance.layout.Alignment
import androidx.glance.layout.Box
import androidx.glance.layout.Column
import androidx.glance.layout.Row
import androidx.glance.layout.fillMaxWidth
import androidx.glance.layout.padding
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextAlign
import androidx.glance.text.TextStyle
import androidx.glance.unit.ColorProvider

/**
 * SPEC-34 §10E habit-chip palette widget.
 *
 * Home-screen Glance widget: tapping a chip fires a one-tap habit capture at the
 * foreground [MeshNodeService] (action [ACTION_CAPTURE_HABIT] + slug), no app
 * cold start. Capture runs in [HabitCaptureAction.onAction] (a plain suspend fn,
 * NOT @Composable) so the chip composable never touches `LocalContext.current`
 * — that inline trips the Compose-compiler IR codegen on this toolchain.
 */
const val ACTION_CAPTURE_HABIT = "ai.phantommesh.app.CAPTURE_HABIT"
const val EXTRA_HABIT_SLUG = "habit_slug"

private val SLUG_KEY = ActionParameters.Key<String>(EXTRA_HABIT_SLUG)

private data class HabitChipSpec(val label: String, val slug: String)

// Slugs MUST match the in-app SPEC-22 STARTER_PALETTE (app/src/lib/captureHabit.ts)
// so widget taps drain into real habit chips instead of coercing to a fallback.
private val HABIT_CHIPS = listOf(
    HabitChipSpec(label = "喝水", slug = "water"),
    HabitChipSpec(label = "咖啡", slug = "coffee"),
    HabitChipSpec(label = "運動", slug = "exercise"),
    HabitChipSpec(label = "深呼吸", slug = "breath"),
)

/** Runs off-composition: builds the capture intent + starts the service. */
class HabitCaptureAction : ActionCallback {
    override suspend fun onAction(
        context: Context,
        glanceId: GlanceId,
        parameters: ActionParameters,
    ) {
        val slug = parameters[SLUG_KEY] ?: "log"
        val intent = Intent(context, MeshNodeService::class.java).also {
            it.action = ACTION_CAPTURE_HABIT
            it.putExtra(EXTRA_HABIT_SLUG, slug)
        }
        context.startForegroundService(intent)
    }
}

class HabitChipPaletteWidget : GlanceAppWidget() {
    override suspend fun provideGlance(context: Context, id: GlanceId) {
        provideContent {
            GlanceTheme { HabitChipPaletteContent() }
        }
    }
}

class HabitChipPaletteWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = HabitChipPaletteWidget()
}

@Composable
private fun HabitChipPaletteContent() {
    Column(
        modifier = GlanceModifier
            .fillMaxWidth()
            .background(ColorProvider(Color(0xFFF8FAFC)))
            .cornerRadius(16.dp)
            .padding(12.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text = "習慣快記",
            style = TextStyle(
                color = ColorProvider(Color(0xFF0F172A)),
                fontWeight = FontWeight.Bold,
                textAlign = TextAlign.Center,
            ),
            modifier = GlanceModifier.fillMaxWidth().padding(bottom = 8.dp),
        )
        Row(modifier = GlanceModifier.fillMaxWidth()) {
            HABIT_CHIPS.take(2).forEach { chip -> HabitChip(chip) }
        }
        Row(modifier = GlanceModifier.fillMaxWidth().padding(top = 8.dp)) {
            HABIT_CHIPS.drop(2).take(2).forEach { chip -> HabitChip(chip) }
        }
    }
}

@Composable
private fun HabitChip(chip: HabitChipSpec) {
    Box(
        modifier = GlanceModifier
            .padding(horizontal = 4.dp)
            .background(ColorProvider(Color(0xFFE0F2FE)))
            .cornerRadius(999.dp)
            .clickable(
                actionRunCallback<HabitCaptureAction>(
                    actionParametersOf(SLUG_KEY to chip.slug)
                )
            )
            .padding(horizontal = 10.dp, vertical = 8.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = chip.label,
            style = TextStyle(
                color = ColorProvider(Color(0xFF075985)),
                fontWeight = FontWeight.Medium,
                textAlign = TextAlign.Center,
            ),
        )
    }
}
