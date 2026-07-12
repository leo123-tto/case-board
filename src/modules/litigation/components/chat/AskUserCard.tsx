/**
 * V0.3 · 选项式追问卡片(Claude Code 风格提问框)。
 *
 * 后端模型调 `ask_user` 工具时,前端把每个问题渲染成可点击的选项 + 可选自由输入框。
 * 用户选完/填完点「提交回答」,把「问 → 答」编号文本当作下一条普通 user 消息回灌,
 * 模型下一轮基于答案续写(大概率这轮就 save_artifact)。
 *
 * 单问题 + 有选项 + 不需自由输入 → 点一下选项即提交(一键,手感最顺)。
 * 多问题 / 需自由输入 → 逐题选/填 + 底部「提交回答」。
 */

import { useState } from "react";
import { CornerDownLeft } from "lucide-react";

import type { AskQuestion } from "@/lib/api";
import { cn } from "@/lib/utils";

interface Props {
  questions: AskQuestion[];
  disabled?: boolean;
  /** 用户提交后回灌的「问→答」文本(已编号);调用方据此发下一条 user 消息 */
  onSubmit: (answerText: string) => void;
}

/** 一题的当前作答:选中的预设项 + 自由输入;自由输入非空时优先生效 */
interface Answer {
  picked: string[];
  text: string;
}

function effective(question: AskQuestion, answer: Answer): string {
  const a = answer;
  const t = a.text.trim();
  if (t) return t;
  const ordered = question.options.filter((option) => a.picked.includes(option));
  return ordered.join("、");
}

function selectionBounds(question: AskQuestion): { min: number; max: number } {
  const optionCount = question.options.length;
  const min = Math.min(Math.max(question.min_selections ?? 1, 0), optionCount);
  const max = Math.min(
    Math.max(question.max_selections ?? optionCount, min),
    optionCount,
  );
  return { min, max };
}

function answered(question: AskQuestion, answer: Answer): boolean {
  if (answer.text.trim()) return true;
  if (question.multiple) return answer.picked.length >= selectionBounds(question).min;
  return answer.picked.length > 0;
}

/** 把所有「问→答」拼成回灌文本:单问省编号,多问编号 */
function composeAnswerText(questions: AskQuestion[], answers: Answer[]): string {
  const pairs = questions.map(
    (q, i) => `${q.question} → ${effective(q, answers[i])}`,
  );
  if (pairs.length === 1) return pairs[0];
  return pairs.map((p, i) => `${i + 1}. ${p}`).join("\n");
}

export function AskUserCard({ questions, disabled, onSubmit }: Props) {
  const [answers, setAnswers] = useState<Answer[]>(() =>
    questions.map(() => ({ picked: [], text: "" })),
  );

  // 单问题 + 有选项 + 不需自由输入 → 一键提交
  const oneClick =
    questions.length === 1 &&
    questions[0].options.length > 0 &&
    !questions[0].allow_input &&
    !questions[0].multiple;

  const allAnswered = answers.every((answer, index) =>
    answered(questions[index], answer),
  );
  const visualizationProposal =
    questions.length === 1 &&
    Boolean(questions[0].multiple) &&
    /可视化|图表|图示|时间线|关系图|思维导图|证据矩阵/.test(
      `${questions[0].question}${questions[0].options.join("")}`,
    );

  function update(i: number, patch: Partial<Answer>) {
    setAnswers((cur) => cur.map((a, idx) => (idx === i ? { ...a, ...patch } : a)));
  }

  function pick(i: number, opt: string) {
    if (disabled) return;
    if (oneClick) {
      // 直接提交,不经 state(单题一键)
      onSubmit(`${questions[i].question} → ${opt}`);
      return;
    }
    setAnswers((current) =>
      current.map((answer, index) => {
        if (index !== i) return answer;
        const selected = answer.picked.includes(opt);
        if (questions[i].multiple) {
          const { max } = selectionBounds(questions[i]);
          if (!selected && answer.picked.length >= max) return answer;
          return {
            ...answer,
            picked: selected
              ? answer.picked.filter((item) => item !== opt)
              : [...answer.picked, opt],
          };
        }
        return { ...answer, picked: selected ? [] : [opt] };
      }),
    );
  }

  function submit() {
    if (disabled || !allAnswered) return;
    onSubmit(composeAnswerText(questions, answers));
  }

  return (
    <div className="mt-2 max-w-[95%] rounded-lg border border-sky-500/30 bg-sky-500/5 px-3 py-2.5 text-sm">
      <p className="mb-2 text-xs font-medium text-sky-700 dark:text-sky-300">
        请选择或填写(点选项即可{oneClick ? "" : ",填完点提交"}):
      </p>
      <div className="space-y-3">
        {questions.map((q, i) => {
          const showInput = q.allow_input || q.options.length === 0;
          const bounds = selectionBounds(q);
          return (
            <div key={i}>
              <p className="mb-1.5 text-foreground">
                {questions.length > 1 && (
                  <span className="mr-1 font-medium text-muted-foreground">
                    {i + 1}.
                  </span>
                )}
                {q.question}
              </p>
              {q.options.length > 0 && (
                <div className="flex flex-wrap gap-1.5">
                  {q.options.map((opt) => (
                    <button
                      key={opt}
                      type="button"
                      aria-pressed={answers[i].picked.includes(opt)}
                      disabled={
                        disabled ||
                        Boolean(
                          q.multiple &&
                            !answers[i].picked.includes(opt) &&
                            answers[i].picked.length >= bounds.max,
                        )
                      }
                      onClick={() => pick(i, opt)}
                      className={cn(
                        "rounded-full border px-2.5 py-1 text-xs transition-colors disabled:opacity-40",
                        answers[i].picked.includes(opt)
                          ? "border-sky-500 bg-sky-500 text-white"
                          : "border-border bg-background hover:border-sky-500/50 hover:bg-sky-500/10",
                      )}
                    >
                      {opt}
                    </button>
                  ))}
                </div>
              )}
              {showInput && (
                <input
                  type="text"
                  value={answers[i].text}
                  disabled={disabled}
                  onChange={(e) => update(i, { text: e.target.value })}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && !oneClick && allAnswered) submit();
                  }}
                  placeholder={
                    q.options.length > 0 ? "或自己输入…" : "请输入…"
                  }
                  className="mt-1.5 w-full rounded-md border border-border bg-background px-2.5 py-1.5 text-xs outline-none transition-[border-color,box-shadow] focus:border-foreground focus:ring-1 focus:ring-foreground/20 disabled:opacity-40"
                />
              )}
            </div>
          );
        })}
      </div>
      {/* 起草追问保留「直接写」出口;可视化建议改成明确跳过,避免把图表选择误回灌成起草指令。 */}
      <div className="mt-3 flex items-center justify-between gap-2">
        <button
          type="button"
          disabled={disabled}
          onClick={() => {
            if (visualizationProposal) {
              onSubmit(`${questions[0].question} → 暂不生成`);
              return;
            }
            onSubmit(
              "信息够了,请直接根据现有信息起草,缺的关键项留 [占位] 待我补充,不用再问了。",
            );
          }}
          className="text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline disabled:opacity-40"
        >
          {visualizationProposal ? "暂不生成" : "信息够了,直接写 →"}
        </button>
        {!oneClick && (
          <button
            type="button"
            disabled={disabled || !allAnswered}
            onClick={submit}
            className="inline-flex items-center gap-1 rounded-md bg-sky-600 px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-sky-700 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <CornerDownLeft className="size-3.5" />
            提交回答
          </button>
        )}
      </div>
    </div>
  );
}
