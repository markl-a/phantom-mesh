import { create } from 'zustand';

export interface Message {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: string;
}

interface ConversationState {
  messages: Message[];
  add: (msg: Message) => void;
  set: (msgs: Message[]) => void;
  remove: (id: string) => void;
  clear: () => void;
  handleEvent: (payload: { type: string; data: Record<string, unknown> }) => void;
}

export const useConversationStore = create<ConversationState>()((set) => ({
  messages: [],
  add: (msg: Message) => set((s: ConversationState) => ({ messages: [...s.messages, msg] })),
  set: (messages: Message[]) => set({ messages }),
  remove: (id: string) => set((s: ConversationState) => ({ messages: s.messages.filter((m: Message) => m.id !== id) })),
  clear: () => set({ messages: [] }),
  handleEvent: (payload: { type: string; data: Record<string, unknown> }) => {
    if (payload.type === 'ConversationMessage') {
      set((s: ConversationState) => ({
        messages: [...s.messages, {
          id: (payload.data.id as string) || crypto.randomUUID(),
          role: (payload.data.role as Message['role']) || 'assistant',
          content: (payload.data.content as string) || '',
          timestamp: (payload.data.timestamp as string) || new Date().toISOString(),
        }],
      }));
    }
  },
}));
