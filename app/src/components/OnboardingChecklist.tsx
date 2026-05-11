import { useState } from 'react';
import { useNavigate } from 'react-router-dom';

interface CheckItem {
  label: string;
  done: boolean;
  link?: string;
}

export default function OnboardingChecklist() {
  const navigate = useNavigate();
  const [dismissed, setDismissed] = useState(() =>
    localStorage.getItem('phantom_mesh_checklist_dismissed') === 'true'
  );

  const items: CheckItem[] = [
    { label: '設定 KeyVault 密碼', done: true },
    { label: '新增至少一個 Provider', done: true },
    { label: '新增第二個 Provider（備援）', done: false, link: '/providers' },
    { label: '連接一台 Mobile Worker', done: false, link: '/cluster' },
    { label: '設定 Search API', done: false, link: '/providers' },
    { label: '發送第一則 Chat 訊息', done: false, link: '/chat' },
  ];

  const doneCount = items.filter(i => i.done).length;

  if (dismissed || doneCount === items.length) return null;

  const handleDismiss = () => {
    localStorage.setItem('phantom_mesh_checklist_dismissed', 'true');
    setDismissed(true);
  };

  return (
    <div className="bg-phantom-card border border-phantom-border rounded-xl p-4 mb-6">
      <div className="flex justify-between items-center mb-3">
        <span className="text-phantom-primary font-semibold text-sm">
          完成設定（{doneCount}/{items.length}）
        </span>
        <button onClick={handleDismiss} className="text-phantom-muted text-xs hover:text-white">
          ✕ 關閉
        </button>
      </div>
      <div className="grid grid-cols-2 gap-2">
        {items.map((item, i) => (
          <div
            key={i}
            onClick={() => item.link && !item.done && navigate(item.link)}
            className={`rounded-lg px-3 py-2 text-xs flex items-center gap-2 ${
              item.done
                ? 'bg-green-900/30 border border-green-800 text-green-400 line-through'
                : 'bg-phantom-bg border border-phantom-border text-phantom-text cursor-pointer hover:border-phantom-primary'
            }`}
          >
            <span>{item.done ? '✓' : '○'}</span>
            <span>{item.label}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
