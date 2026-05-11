import { useState, useEffect, useCallback } from "react";
import { safeInvoke as invoke } from "../lib/tauri-compat";
import {
  Target, Plus, ChevronRight, CheckCircle2, Circle, Flame,
  Calendar, TrendingUp, MessageCircle, BarChart3, SmilePlus,
} from "lucide-react";

interface Goal {
  id: string;
  title: string;
  category: string;
  description: string | null;
  target_date: string | null;
  status: string;
  context: string | null;
  created_at: string;
  updated_at: string;
}

interface Milestone {
  id: string;
  goal_id: string;
  title: string;
  due_date: string | null;
  status: string;
  sort_order: number;
  completed_at: string | null;
}

interface RecurringTask {
  id: string;
  goal_id: string;
  title: string;
  cron_expr: string;
  last_completed: string | null;
  streak_count: number;
  enabled: boolean;
}

interface TodayTask {
  task: RecurringTask;
  goal_title: string;
  completed_today: boolean;
}

interface CheckIn {
  id: string;
  goal_id: string;
  date: string;
  mood: number;
  note: string | null;
  ai_feedback: string | null;
}

interface GoalProgress {
  goal: Goal;
  milestones_total: number;
  milestones_done: number;
  percentage: number;
  current_streak: number;
  days_remaining: number | null;
  recent_check_ins: CheckIn[];
}

interface WeeklySummary {
  week_start: string;
  week_end: string;
  total_tasks: number;
  completed_tasks: number;
  completion_rate: number;
  avg_mood: number | null;
  best_streak: number;
  milestones_completed: number;
}

interface MoodPoint {
  date: string;
  mood?: number;
  avg_mood?: number;
}

const STATUS_COLORS: Record<string, string> = {
  active: "bg-phantom-primary/20 text-phantom-primary",
  completed: "bg-phantom-success/20 text-phantom-success",
  paused: "bg-phantom-warning/20 text-phantom-warning",
  abandoned: "bg-phantom-muted/20 text-phantom-muted",
};

const STATUS_LABELS: Record<string, string> = {
  active: "進行中",
  completed: "已完成",
  paused: "暫停",
  abandoned: "已放棄",
};

const MOOD_EMOJIS = ["", "😢", "😔", "😐", "😊", "🤩"];
const MOOD_LABELS = ["", "糟糕", "不太好", "普通", "不錯", "超棒"];
const MOOD_COLORS = ["", "bg-red-400", "bg-orange-400", "bg-yellow-400", "bg-green-400", "bg-emerald-400"];

export default function Goals() {
  const [goals, setGoals] = useState<Goal[]>([]);
  const [todayTasks, setTodayTasks] = useState<TodayTask[]>([]);
  const [selectedGoal, setSelectedGoal] = useState<string | null>(null);
  const [progress, setProgress] = useState<GoalProgress | null>(null);
  const [milestones, setMilestones] = useState<Milestone[]>([]);
  const [recurringTasks, setRecurringTasks] = useState<RecurringTask[]>([]);
  const [checkIns, setCheckIns] = useState<CheckIn[]>([]);
  const [moodTrend, setMoodTrend] = useState<MoodPoint[]>([]);
  const [weeklySummary, setWeeklySummary] = useState<WeeklySummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // New goal form
  const [showNewGoal, setShowNewGoal] = useState(false);
  const [newTitle, setNewTitle] = useState("");
  const [newCategory, setNewCategory] = useState("");
  const [newDescription, setNewDescription] = useState("");
  const [newTargetDate, setNewTargetDate] = useState("");

  // Check-in dialog
  const [showCheckIn, setShowCheckIn] = useState(false);
  const [checkInMood, setCheckInMood] = useState(3);
  const [checkInNote, setCheckInNote] = useState("");
  const [submittingCheckIn, setSubmittingCheckIn] = useState(false);

  const fetchGoals = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [goalsRes, todayRes, weeklyRes] = await Promise.all([
        invoke<{ goals?: Goal[] }>("goals_list", { status: "active" }).catch(() => ({ goals: [] })),
        invoke<{ tasks?: TodayTask[] }>("goals_today").catch(() => ({ tasks: [] })),
        invoke<{ summary?: WeeklySummary }>("goals_weekly_summary").catch(() => ({ summary: null })),
      ]);
      setGoals(goalsRes.goals ?? []);
      setTodayTasks(todayRes.tasks ?? []);
      setWeeklySummary(weeklyRes.summary ?? null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void fetchGoals();
  }, [fetchGoals]);

  const selectGoal = async (id: string) => {
    setSelectedGoal(id);
    try {
      const [progressRes, msRes, taskRes, ciRes, trendRes] = await Promise.all([
        invoke<{ progress?: GoalProgress }>("goals_progress", { id }),
        invoke<{ milestones?: Milestone[] }>("goals_milestones", { id }),
        invoke<{ tasks?: RecurringTask[] }>("goals_recurring_tasks", { id }),
        invoke<{ check_ins?: CheckIn[] }>("goals_checkins", { id, limit: 14 }),
        invoke<{ trend?: MoodPoint[] }>("goals_mood_trend", { id, days: 14 }),
      ]);
      setProgress(progressRes.progress ?? null);
      setMilestones(msRes.milestones ?? []);
      setRecurringTasks(taskRes.tasks ?? []);
      setCheckIns(ciRes.check_ins ?? []);
      setMoodTrend(trendRes.trend ?? []);
    } catch (e) {
      setError(String(e));
    }
  };

  const createGoal = async () => {
    if (!newTitle.trim()) return;
    try {
      await invoke("goals_create", {
        data: {
          title: newTitle,
          category: newCategory || "general",
          description: newDescription || null,
          target_date: newTargetDate || null,
        },
      });
      setNewTitle("");
      setNewCategory("");
      setNewDescription("");
      setNewTargetDate("");
      setShowNewGoal(false);
      await fetchGoals();
    } catch (e) {
      setError(String(e));
    }
  };

  const toggleMilestone = async (goalId: string, msId: string) => {
    try {
      await invoke("goals_milestone_toggle", { goalId, milestoneId: msId });
      await selectGoal(goalId);
    } catch (e) {
      setError(String(e));
    }
  };

  const completeRecurring = async (goalId: string, taskId: string) => {
    try {
      await invoke("goals_recurring_complete", { goalId, taskId });
      await Promise.all([fetchGoals(), selectGoal(goalId)]);
    } catch (e) {
      setError(String(e));
    }
  };

  const submitCheckIn = async () => {
    if (!selectedGoal || submittingCheckIn) return;
    setSubmittingCheckIn(true);
    try {
      await invoke("goals_checkin_add", {
        goalId: selectedGoal,
        data: {
          mood: checkInMood,
          note: checkInNote || null,
        },
      });
      setShowCheckIn(false);
      setCheckInMood(3);
      setCheckInNote("");
      await selectGoal(selectedGoal);
    } catch (e) {
      setError(String(e));
    } finally {
      setSubmittingCheckIn(false);
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center py-16">
        <div className="w-6 h-6 border-2 border-phantom-primary border-t-transparent rounded-full animate-spin" />
        <span className="ml-3 text-phantom-muted text-sm">載入目標資料...</span>
      </div>
    );
  }

  return (
    <div className="flex gap-6 h-full">
      {/* Left: Goal list + Today's tasks */}
      <div className="w-80 flex-shrink-0 space-y-4">
        {/* Weekly summary card */}
        {weeklySummary && weeklySummary.total_tasks > 0 && (
          <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
            <h3 className="text-sm font-medium text-phantom-text mb-3 flex items-center gap-2">
              <BarChart3 size={14} />
              本週概覽
            </h3>
            <div className="grid grid-cols-2 gap-3">
              <div className="text-center">
                <p className="text-lg font-bold text-phantom-primary">
                  {weeklySummary.completion_rate.toFixed(0)}%
                </p>
                <p className="text-[10px] text-phantom-muted">完成率</p>
              </div>
              <div className="text-center">
                <p className="text-lg font-bold text-phantom-warning">
                  {weeklySummary.best_streak > 0 ? `${weeklySummary.best_streak}d` : "-"}
                </p>
                <p className="text-[10px] text-phantom-muted">最佳連續</p>
              </div>
              {weeklySummary.avg_mood !== null && (
                <div className="text-center">
                  <p className="text-lg">{MOOD_EMOJIS[Math.round(weeklySummary.avg_mood)] || "😐"}</p>
                  <p className="text-[10px] text-phantom-muted">平均心情</p>
                </div>
              )}
              {weeklySummary.milestones_completed > 0 && (
                <div className="text-center">
                  <p className="text-lg font-bold text-phantom-success">{weeklySummary.milestones_completed}</p>
                  <p className="text-[10px] text-phantom-muted">里程碑</p>
                </div>
              )}
            </div>
          </div>
        )}

        {/* Today's tasks */}
        {todayTasks.length > 0 && (
          <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
            <h3 className="text-sm font-medium text-phantom-text mb-3 flex items-center gap-2">
              <Calendar size={14} />
              今日任務
              <span className="text-xs text-phantom-muted ml-auto">
                {todayTasks.filter(t => t.completed_today).length}/{todayTasks.length}
              </span>
            </h3>
            <div className="space-y-2">
              {todayTasks.map((tt) => (
                <button
                  key={tt.task.id}
                  onClick={() => {
                    if (!tt.completed_today) {
                      void completeRecurring(tt.task.goal_id, tt.task.id);
                    }
                  }}
                  className={`w-full text-left flex items-center gap-2 px-3 py-2 rounded text-xs transition ${
                    tt.completed_today
                      ? "bg-phantom-success/10 text-phantom-muted line-through"
                      : "bg-phantom-bg border border-phantom-border hover:border-phantom-primary/50"
                  }`}
                >
                  {tt.completed_today ? <CheckCircle2 size={14} className="text-phantom-success" /> : <Circle size={14} />}
                  <div className="flex-1 min-w-0">
                    <span className="block truncate">{tt.task.title}</span>
                    <span className="block text-phantom-muted truncate">{tt.goal_title}</span>
                  </div>
                  {tt.task.streak_count > 0 && (
                    <span className="flex items-center gap-0.5 text-phantom-warning">
                      <Flame size={12} />
                      {tt.task.streak_count}
                    </span>
                  )}
                </button>
              ))}
            </div>
          </div>
        )}

        {/* Goal list */}
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
          <div className="flex items-center justify-between mb-3">
            <h3 className="text-sm font-medium text-phantom-text flex items-center gap-2">
              <Target size={14} />
              我的目標
            </h3>
            <button
              onClick={() => setShowNewGoal(!showNewGoal)}
              className="text-phantom-primary hover:text-phantom-primary/80 transition"
            >
              <Plus size={16} />
            </button>
          </div>

          {/* New goal form */}
          {showNewGoal && (
            <div className="bg-phantom-bg border border-phantom-border rounded-lg p-3 mb-3 space-y-2">
              <input
                value={newTitle}
                onChange={(e) => setNewTitle(e.target.value)}
                placeholder="目標名稱"
                className="w-full bg-phantom-card border border-phantom-border rounded px-3 py-1.5 text-sm text-phantom-text placeholder-phantom-muted focus:outline-none focus:border-phantom-primary"
              />
              <input
                value={newCategory}
                onChange={(e) => setNewCategory(e.target.value)}
                placeholder="分類（如：學業、健康、事業）"
                className="w-full bg-phantom-card border border-phantom-border rounded px-3 py-1.5 text-sm text-phantom-text placeholder-phantom-muted focus:outline-none focus:border-phantom-primary"
              />
              <textarea
                value={newDescription}
                onChange={(e) => setNewDescription(e.target.value)}
                placeholder="描述（選填）"
                rows={2}
                className="w-full bg-phantom-card border border-phantom-border rounded px-3 py-1.5 text-sm text-phantom-text placeholder-phantom-muted focus:outline-none focus:border-phantom-primary resize-none"
              />
              <input
                type="date"
                value={newTargetDate}
                onChange={(e) => setNewTargetDate(e.target.value)}
                className="w-full bg-phantom-card border border-phantom-border rounded px-3 py-1.5 text-sm text-phantom-text focus:outline-none focus:border-phantom-primary"
              />
              <div className="flex gap-2">
                <button
                  onClick={createGoal}
                  disabled={!newTitle.trim()}
                  className="flex-1 bg-phantom-primary text-phantom-bg py-1.5 rounded text-sm font-medium hover:brightness-110 disabled:opacity-40"
                >
                  建立
                </button>
                <button
                  onClick={() => setShowNewGoal(false)}
                  className="px-3 py-1.5 rounded text-sm text-phantom-muted hover:bg-phantom-bg"
                >
                  取消
                </button>
              </div>
            </div>
          )}

          {/* Goal items */}
          {goals.length === 0 ? (
            <p className="text-xs text-phantom-muted py-4 text-center">
              還沒有目標。從對話中說出你的目標，或點擊 + 新增。
            </p>
          ) : (
            <div className="space-y-1.5">
              {goals.map((g) => (
                <button
                  key={g.id}
                  onClick={() => void selectGoal(g.id)}
                  className={`w-full text-left px-3 py-2.5 rounded transition ${
                    selectedGoal === g.id
                      ? "bg-phantom-primary/15 border border-phantom-primary/30"
                      : "hover:bg-phantom-bg border border-transparent"
                  }`}
                >
                  <div className="flex items-center justify-between">
                    <span className="text-sm text-phantom-text truncate">{g.title}</span>
                    <ChevronRight size={14} className="text-phantom-muted flex-shrink-0" />
                  </div>
                  <div className="flex items-center gap-2 mt-1">
                    <span className={`inline-block px-1.5 py-0.5 rounded text-[10px] font-medium ${STATUS_COLORS[g.status] ?? STATUS_COLORS.active}`}>
                      {STATUS_LABELS[g.status] ?? g.status}
                    </span>
                    {g.category && (
                      <span className="text-[10px] text-phantom-muted">{g.category}</span>
                    )}
                  </div>
                </button>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* Right: Goal detail */}
      <div className="flex-1 min-w-0">
        {error && (
          <div className="bg-phantom-danger/20 border border-phantom-danger rounded p-3 mb-4 text-sm">
            {error}
          </div>
        )}

        {!selectedGoal ? (
          <div className="flex flex-col items-center justify-center h-full text-phantom-muted">
            <Target size={48} className="mb-4 opacity-30" />
            <p className="text-sm">選擇一個目標查看詳情</p>
            <p className="text-xs mt-1">或在對話中告訴 Phantom 你的目標</p>
          </div>
        ) : progress ? (
          <div className="space-y-6">
            {/* Goal header */}
            <div>
              <div className="flex items-start justify-between">
                <div>
                  <h2 className="text-xl font-bold text-phantom-text">{progress.goal.title}</h2>
                  {progress.goal.description && (
                    <p className="text-sm text-phantom-muted mt-1">{progress.goal.description}</p>
                  )}
                </div>
                <button
                  onClick={() => setShowCheckIn(true)}
                  className="flex items-center gap-1.5 px-3 py-1.5 bg-phantom-primary/15 text-phantom-primary rounded text-sm hover:bg-phantom-primary/25 transition"
                >
                  <SmilePlus size={14} />
                  記錄心情
                </button>
              </div>
              <div className="flex items-center gap-4 mt-3">
                <span className={`inline-block px-2 py-0.5 rounded text-xs font-medium ${STATUS_COLORS[progress.goal.status] ?? STATUS_COLORS.active}`}>
                  {STATUS_LABELS[progress.goal.status] ?? progress.goal.status}
                </span>
                {progress.days_remaining !== null && (
                  <span className="text-xs text-phantom-muted flex items-center gap-1">
                    <Calendar size={12} />
                    {progress.days_remaining > 0 ? `還有 ${progress.days_remaining} 天` : "已到期"}
                  </span>
                )}
                {progress.current_streak > 0 && (
                  <span className="text-xs text-phantom-warning flex items-center gap-1">
                    <Flame size={12} />
                    連續 {progress.current_streak} 天
                  </span>
                )}
              </div>
            </div>

            {/* Check-in dialog */}
            {showCheckIn && (
              <div className="bg-phantom-card border border-phantom-primary/30 rounded-lg p-4">
                <h3 className="text-sm font-medium text-phantom-text mb-3">今天這個目標進展如何？</h3>
                <div className="flex items-center justify-center gap-3 mb-4">
                  {[1, 2, 3, 4, 5].map((m) => (
                    <button
                      key={m}
                      onClick={() => setCheckInMood(m)}
                      className={`flex flex-col items-center gap-1 px-3 py-2 rounded-lg transition ${
                        checkInMood === m
                          ? "bg-phantom-primary/20 ring-2 ring-phantom-primary"
                          : "hover:bg-phantom-bg"
                      }`}
                    >
                      <span className="text-2xl">{MOOD_EMOJIS[m]}</span>
                      <span className="text-[10px] text-phantom-muted">{MOOD_LABELS[m]}</span>
                    </button>
                  ))}
                </div>
                <textarea
                  value={checkInNote}
                  onChange={(e) => setCheckInNote(e.target.value)}
                  placeholder="今天的心得或筆記（選填）"
                  rows={2}
                  className="w-full bg-phantom-bg border border-phantom-border rounded px-3 py-2 text-sm text-phantom-text placeholder-phantom-muted focus:outline-none focus:border-phantom-primary resize-none mb-3"
                />
                <div className="flex gap-2">
                  <button
                    onClick={submitCheckIn}
                    disabled={submittingCheckIn}
                    className="flex-1 bg-phantom-primary text-phantom-bg py-1.5 rounded text-sm font-medium hover:brightness-110 disabled:opacity-40"
                  >
                    {submittingCheckIn ? "送出中..." : "記錄"}
                  </button>
                  <button
                    onClick={() => setShowCheckIn(false)}
                    className="px-3 py-1.5 rounded text-sm text-phantom-muted hover:bg-phantom-bg"
                  >
                    取消
                  </button>
                </div>
              </div>
            )}

            {/* Progress bar */}
            <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
              <div className="flex items-center justify-between mb-2">
                <span className="text-sm text-phantom-text flex items-center gap-2">
                  <TrendingUp size={14} />
                  進度
                </span>
                <span className="text-sm font-medium text-phantom-primary">
                  {progress.percentage.toFixed(0)}%
                </span>
              </div>
              <div className="w-full h-2 bg-phantom-bg rounded-full overflow-hidden">
                <div
                  className="h-full bg-phantom-primary rounded-full transition-all duration-500"
                  style={{ width: `${Math.min(progress.percentage, 100)}%` }}
                />
              </div>
              <p className="text-xs text-phantom-muted mt-2">
                {progress.milestones_done} / {progress.milestones_total} 里程碑完成
              </p>
            </div>

            {/* Mood trend chart */}
            {moodTrend.length > 0 && (
              <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
                <h3 className="text-sm font-medium text-phantom-text mb-3 flex items-center gap-2">
                  <BarChart3 size={14} />
                  心情趨勢（近 14 天）
                </h3>
                <div className="flex items-end gap-1 h-16">
                  {[...moodTrend].reverse().map((p, i) => {
                    const mood = p.mood ?? 3;
                    const h = (mood / 5) * 100;
                    return (
                      <div
                        key={i}
                        className="flex-1 flex flex-col items-center group relative"
                      >
                        <div
                          className={`w-full rounded-t ${MOOD_COLORS[mood] || "bg-gray-400"} transition-all`}
                          style={{ height: `${h}%` }}
                        />
                        <div className="hidden group-hover:block absolute -top-8 bg-phantom-bg border border-phantom-border rounded px-2 py-0.5 text-[10px] text-phantom-text whitespace-nowrap z-10">
                          {p.date} {MOOD_EMOJIS[mood]}
                        </div>
                      </div>
                    );
                  })}
                </div>
                <div className="flex justify-between mt-1">
                  <span className="text-[10px] text-phantom-muted">
                    {moodTrend[moodTrend.length - 1]?.date ?? ""}
                  </span>
                  <span className="text-[10px] text-phantom-muted">
                    {moodTrend[0]?.date ?? ""}
                  </span>
                </div>
              </div>
            )}

            {/* Milestones */}
            <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
              <h3 className="text-sm font-medium text-phantom-text mb-3">里程碑</h3>
              {milestones.length === 0 ? (
                <p className="text-xs text-phantom-muted">尚未建立里程碑 — 在對話中請 Phantom 為你規劃</p>
              ) : (
                <div className="space-y-2">
                  {milestones.map((ms) => (
                    <button
                      key={ms.id}
                      onClick={() => void toggleMilestone(progress.goal.id, ms.id)}
                      className="w-full text-left flex items-center gap-3 px-3 py-2 rounded hover:bg-phantom-bg transition"
                    >
                      {ms.status === "done" ? (
                        <CheckCircle2 size={16} className="text-phantom-success flex-shrink-0" />
                      ) : (
                        <Circle size={16} className="text-phantom-muted flex-shrink-0" />
                      )}
                      <span className={`text-sm ${ms.status === "done" ? "line-through text-phantom-muted" : "text-phantom-text"}`}>
                        {ms.title}
                      </span>
                      {ms.due_date && (
                        <span className="text-[10px] text-phantom-muted ml-auto">{ms.due_date}</span>
                      )}
                    </button>
                  ))}
                </div>
              )}
            </div>

            {/* Recurring tasks */}
            <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
              <h3 className="text-sm font-medium text-phantom-text mb-3">每日任務</h3>
              {recurringTasks.length === 0 ? (
                <p className="text-xs text-phantom-muted">尚未建立每日任務 — 在對話中請 Phantom 為你安排</p>
              ) : (
                <div className="space-y-2">
                  {recurringTasks.map((t) => {
                    const today = new Date().toISOString().split("T")[0];
                    const doneToday = t.last_completed === today;
                    return (
                      <button
                        key={t.id}
                        onClick={() => {
                          if (!doneToday) void completeRecurring(progress.goal.id, t.id);
                        }}
                        className={`w-full text-left flex items-center gap-3 px-3 py-2 rounded transition ${
                          doneToday ? "bg-phantom-success/10" : "hover:bg-phantom-bg"
                        }`}
                      >
                        {doneToday ? (
                          <CheckCircle2 size={16} className="text-phantom-success flex-shrink-0" />
                        ) : (
                          <Circle size={16} className="text-phantom-muted flex-shrink-0" />
                        )}
                        <span className={`text-sm flex-1 ${doneToday ? "line-through text-phantom-muted" : "text-phantom-text"}`}>
                          {t.title}
                        </span>
                        {t.streak_count > 0 && (
                          <span className="text-xs text-phantom-warning flex items-center gap-0.5">
                            <Flame size={12} />
                            {t.streak_count}
                          </span>
                        )}
                      </button>
                    );
                  })}
                </div>
              )}
            </div>

            {/* Recent check-ins */}
            {checkIns.length > 0 && (
              <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
                <h3 className="text-sm font-medium text-phantom-text mb-3 flex items-center gap-2">
                  <MessageCircle size={14} />
                  最近 Check-in
                </h3>
                <div className="space-y-3">
                  {checkIns.slice(0, 7).map((ci) => (
                    <div key={ci.id} className="flex items-start gap-3 px-2">
                      <span className="text-lg flex-shrink-0">{MOOD_EMOJIS[ci.mood] || "😐"}</span>
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="text-xs text-phantom-muted">{ci.date}</span>
                          <span className="text-[10px] text-phantom-muted">{MOOD_LABELS[ci.mood]}</span>
                        </div>
                        {ci.note && (
                          <p className="text-xs text-phantom-text mt-0.5">{ci.note}</p>
                        )}
                        {ci.ai_feedback && (
                          <p className="text-xs text-phantom-primary/80 mt-0.5 italic">{ci.ai_feedback}</p>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
        ) : (
          <div className="flex items-center justify-center py-16">
            <div className="w-5 h-5 border-2 border-phantom-primary border-t-transparent rounded-full animate-spin" />
          </div>
        )}
      </div>
    </div>
  );
}
