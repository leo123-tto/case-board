import { useState } from "react";
import { CornerDownLeft } from "lucide-react";

import { cn } from "@/lib/utils";

export interface AskUserQuestion {
  question: string;
  options: string[];
  allow_input?: boolean;
  multiple?: boolean;
  min_selections?: number | null;
  max_selections?: number | null;
}

interface Props {
  questions: AskUserQuestion[];
  disabled?: boolean;
  onSubmit: (answerText: string) => void;
}

interface Answer {
  picked: string[];
  text: string;
}

function selectionBounds(question: AskUserQuestion): { min: number; max: number } {
  const optionCount = question.options.length;
  const min = Math.min(Math.max(question.min_selections ?? 1, 0), optionCount);
  const max = Math.min(
    Math.max(question.max_selections ?? optionCount, min),
    optionCount,
  );
  return { min, max };
}

function effective(question: AskUserQuestion, answer: Answer): string {
  const text = answer.text.trim();
  const ordered = question.options.filter((option) =>
    answer.picked.includes(option),
  );
  if (question.multiple && ordered.length > 0 && text) {
    return `${ordered.join("、")}；补充：${text}`;
  }
  return text || ordered.join("、");
}

function answered(question: AskUserQuestion, answer: Answer): boolean {
  if (answer.text.trim()) return true;
  if (question.multiple) {
    return answer.picked.length >= selectionBounds(question).min;
  }
  return answer.picked.length > 0;
}

function composeAnswerText(
  questions: AskUserQuestion[],
  answers: Answer[],
): string {
  const pairs = questions.map(
    (question, index) =>
      `${question.question} → ${effective(question, answers[index])}`,
  );
  if (pairs.length === 1) return pairs[0];
  return pairs.map((pair, index) => `${index + 1}. ${pair}`).join("\n");
}

export function AskUserCard({ questions, disabled, onSubmit }: Props) {
  const [answers, setAnswers] = useState<Answer[]>(() =>
    questions.map(() => ({ picked: [], text: "" })),
  );

  const oneClick =
    questions.length === 1 &&
    questions[0].options.length > 0 &&
    !questions[0].multiple;
  const allAnswered = answers.every((answer, index) =>
    answered(questions[index], answer),
  );
  const hasCustomText = answers.some((answer) => Boolean(answer.text.trim()));
  const visualizationProposal =
    questions.length === 1 &&
    Boolean(questions[0].multiple) &&
    /可视化|图表|图示|时间线|关系图|思维导图|证据矩阵/.test(
      `${questions[0].question}${questions[0].options.join("")}`,
    );

  function update(index: number, patch: Partial<Answer>) {
    setAnswers((current) =>
      current.map((answer, answerIndex) =>
        answerIndex === index ? { ...answer, ...patch } : answer,
      ),
    );
  }

  function updateCustomAnswer(index: number, text: string) {
    update(
      index,
      questions[index].multiple
        ? { text }
        : {
            picked: [],
            text,
          },
    );
  }

  function pick(index: number, option: string) {
    if (disabled) return;
    const question = questions[index];
    if (!question.multiple && answers[index].text.trim()) return;
    if (oneClick) {
      onSubmit(`${question.question} → ${option}`);
      return;
    }
    setAnswers((current) =>
      current.map((answer, answerIndex) => {
        if (answerIndex !== index) return answer;
        const selected = answer.picked.includes(option);
        if (question.multiple) {
          const { max } = selectionBounds(question);
          if (!selected && answer.picked.length >= max) return answer;
          return {
            ...answer,
            picked: selected
              ? answer.picked.filter((item) => item !== option)
              : [...answer.picked, option],
          };
        }
        return {
          picked: selected ? [] : [option],
          text: "",
        };
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
        {oneClick
          ? "请选择一个选项，或输入自己的回答："
          : "请选择或填写，完成后提交："}
      </p>
      <div className="space-y-3">
        {questions.map((question, index) => {
          const bounds = selectionBounds(question);
          const customSingleAnswer = Boolean(
            !question.multiple && answers[index].text.trim(),
          );
          return (
            <div key={index}>
              <p className="mb-1.5 text-foreground">
                {questions.length > 1 ? (
                  <span className="mr-1 font-medium text-muted-foreground">
                    {index + 1}.
                  </span>
                ) : null}
                {question.question}
              </p>
              {question.options.length > 0 ? (
                <div className="flex flex-wrap gap-1.5">
                  {question.options.map((option) => {
                    const selected = answers[index].picked.includes(option);
                    const reachedMultiMaximum =
                      Boolean(question.multiple) &&
                      !selected &&
                      answers[index].picked.length >= bounds.max;
                    return (
                      <button
                        key={option}
                        type="button"
                        aria-pressed={selected}
                        disabled={
                          disabled ||
                          customSingleAnswer ||
                          reachedMultiMaximum
                        }
                        onClick={() => pick(index, option)}
                        className={cn(
                          "rounded-full border px-2.5 py-1 text-xs transition-colors disabled:opacity-40",
                          selected
                            ? "border-sky-500 bg-sky-500 text-white"
                            : "border-border bg-background hover:border-sky-500/50 hover:bg-sky-500/10",
                        )}
                      >
                        {option}
                      </button>
                    );
                  })}
                </div>
              ) : null}
              <input
                type="text"
                aria-label={`自定义回答：${question.question}`}
                value={answers[index].text}
                disabled={disabled}
                onChange={(event) =>
                  updateCustomAnswer(index, event.target.value)
                }
                onKeyDown={(event) => {
                  if (event.key === "Enter" && allAnswered) {
                    event.preventDefault();
                    submit();
                  }
                }}
                placeholder={
                  question.multiple
                    ? "可补充自己的回答…"
                    : "或输入自己的回答…"
                }
                className="mt-1.5 w-full rounded-md border border-border bg-background px-2.5 py-1.5 text-xs outline-none transition-[border-color,box-shadow] focus:border-foreground focus:ring-1 focus:ring-foreground/20 disabled:opacity-40"
              />
            </div>
          );
        })}
      </div>
      <div className="mt-3 flex items-center justify-between gap-2">
        <button
          type="button"
          disabled={disabled || hasCustomText}
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
        <button
          type="button"
          disabled={disabled || !allAnswered}
          onClick={submit}
          className="inline-flex items-center gap-1 rounded-md bg-sky-600 px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-sky-700 disabled:cursor-not-allowed disabled:opacity-40"
        >
          <CornerDownLeft className="size-3.5" />
          {oneClick ? "提交自定义回答" : "提交回答"}
        </button>
      </div>
    </div>
  );
}
