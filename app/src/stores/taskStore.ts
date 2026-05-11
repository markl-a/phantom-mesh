import { create } from 'zustand';
import type { TaskStatus } from '../lib/types';

export interface TaskEntry {
  id: string;
  title: string;
  status: TaskStatus;
  result?: string;
  createdAt: string;
  updatedAt: string;
  parentTaskId?: string;
  retryCount: number;
}

interface TaskState {
  tasks: TaskEntry[];
  add: (task: TaskEntry) => void;
  update: (id: string, partial: Partial<TaskEntry>) => void;
  remove: (id: string) => void;
  set: (tasks: TaskEntry[]) => void;
  handleEvent: (payload: { type: string; data: Record<string, unknown> }) => void;
}

export const useTaskStore = create<TaskState>()((set) => ({
  tasks: [],
  add: (task: TaskEntry) => set((s: TaskState) => ({ tasks: [task, ...s.tasks] })),
  update: (id: string, partial: Partial<TaskEntry>) =>
    set((s: TaskState) => ({
      tasks: s.tasks.map((t: TaskEntry) => (t.id === id ? { ...t, ...partial } : t)),
    })),
  remove: (id: string) => set((s: TaskState) => ({ tasks: s.tasks.filter((t: TaskEntry) => t.id !== id) })),
  set: (tasks: TaskEntry[]) => set({ tasks }),
  handleEvent: (payload: { type: string; data: Record<string, unknown> }) => {
    const { type, data } = payload;
    if (type === 'TaskCreated') {
      set((s: TaskState) => ({
        tasks: [
          {
            id: data.task_id as string,
            title: (data.title as string) || '',
            status: 'pending' as TaskStatus,
            createdAt: (data.timestamp as string) || new Date().toISOString(),
            updatedAt: (data.timestamp as string) || new Date().toISOString(),
            retryCount: 0,
          },
          ...s.tasks,
        ],
      }));
    } else if (type === 'TaskCompleted' || type === 'TaskFailed') {
      set((s: TaskState) => ({
        tasks: s.tasks.map((t: TaskEntry) =>
          t.id === (data.task_id as string)
            ? { ...t, status: (type === 'TaskCompleted' ? 'done' : 'failed') as TaskStatus, result: data.result as string, updatedAt: new Date().toISOString() }
            : t
        ),
      }));
    }
  },
}));
