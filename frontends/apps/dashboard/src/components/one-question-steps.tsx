"use client";

import { startTransition, useActionState, useState } from "react";

import { Button, FormField, Input } from "@vaam-apps/ui";

import { NO_ERROR, type FormAction } from "../form-state";
import { FormAlert } from "./form-alert";

/** One thing the person is asked, and what makes the answer acceptable. */
export interface Question {
  /** The field name the server action reads, and the key in the answers. */
  readonly name: string;
  /** The question itself, as the input's visible label. */
  readonly label: string;
  /** One line of help, or nothing. */
  readonly hint?: string;
  readonly type: "email" | "password" | "text";
  /** The HTML `autocomplete` token, so a password manager knows what this is. */
  readonly autoComplete: string;
  readonly inputMode?: "numeric";
  readonly maxLength?: number;
  /**
   * Whether `value` answers the question, given every answer so far.
   *
   * **This is a convenience and never a check.** It decides whether Continue
   * is enabled so a person is not offered a button that cannot work; vpay
   * decides whether the answer is *right*, and a rule copied here that drifted
   * from the server's would refuse something vpay accepts.
   */
  readonly isValid: (
    value: string,
    answers: Readonly<Record<string, string>>,
  ) => boolean;
}

export interface OneQuestionStepsProps {
  /** A server action from `src/server/actions.ts`, passed in so this file imports no server code. */
  action: FormAction;
  /** In order. One is a single screen with no Back; two or more is a stepper. */
  questions: readonly Question[];
  /** The last step's button. */
  submitLabel: string;
  /** The last step's button while the action runs. */
  pendingLabel: string;
  /**
   * The name of an earlier question whose answer is the account name, so a
   * later password step can carry it for a password manager. Without it the
   * manager sees a password field alone and offers to save the password
   * against nothing.
   */
  usernameFrom?: string;
}

/**
 * Asks one question per screen, and posts every answer once the last is given.
 *
 * # Why this exists rather than a form
 *
 * The owner's ruling, on a sibling product's become-a-vendor screen:
 * *"I banned forms from this app. One screen should collect a single
 * information."* The same rule is applied here, so the email and the password
 * are two screens and the password change is three — current, new, repeat.
 * Navigating between fields on a phone is where the errors come from, and a
 * dead button cannot say which of three fields is wrong; on a screen with one
 * visibly empty field it can only mean that one.
 *
 * * **Continue is disabled until the answer is valid** ({@link Question.isValid}).
 * * **Back is there from the second question on**, and answers are kept both
 *   ways, so a refused password can be followed by a corrected email.
 * * **Enter advances** and, on the last question, submits: the markup is one
 *   `<form>` whatever the step, so the keyboard does what the button does.
 * * **Nothing is posted until the end.** The answers live in this component's
 *   state and reach the server action as one `FormData`, so the server sees
 *   what a single form would have sent and its checks are unchanged.
 *
 * # What it costs
 *
 * It needs JavaScript. The page the server renders without it is a form with
 * one input and a button that does nothing, which is worse than no form; a
 * stepper that advances on the server would need the answers held between
 * requests — in a cookie, for the sign-in, which is a credential — and that is
 * a larger change than the rule justifies.
 *
 * # Why the action is not a `<form action>`
 *
 * React 19 resets an uncontrolled form after its action settles, which would
 * empty the field a refused password was typed into. Every input here is
 * controlled, and the action is called inside a transition from `onSubmit`, so
 * nothing resets and the person corrects rather than retypes.
 */
export function OneQuestionSteps({
  action,
  questions,
  submitLabel,
  pendingLabel,
  usernameFrom,
}: OneQuestionStepsProps) {
  const [state, dispatch, pending] = useActionState(action, NO_ERROR);
  const [step, setStep] = useState(0);
  const [answers, setAnswers] = useState<Record<string, string>>({});

  // A refusal that names its question takes the person back to it. Adjusted
  // during render, on the render that first sees the new state, rather than in
  // an effect: an effect would paint the wrong question for a frame first.
  const [seen, setSeen] = useState(state);
  if (state !== seen) {
    setSeen(state);
    const target = questions.findIndex((asked) => asked.name === state.field);
    if (state.error !== null && target >= 0) {
      setStep(target);
    }
  }

  const question = questions[step];
  if (question === undefined) {
    return null;
  }
  const value = answers[question.name] ?? "";
  const valid = question.isValid(value, answers);
  const last = step === questions.length - 1;
  // Which question the refusal belongs to: the one it names, or the last when
  // it names none (or one that is not asked here).
  const named = questions.findIndex((asked) => asked.name === state.field);
  const owner = named >= 0 ? named : questions.length - 1;
  const here = step === owner;
  const invalid = state.error !== null && here;
  const carried =
    usernameFrom !== undefined && step > 0 ? answers[usernameFrom] : undefined;

  function advance() {
    if (!valid || pending) {
      return;
    }
    if (!last) {
      setStep(step + 1);
      return;
    }
    const form = new FormData();
    for (const asked of questions) {
      form.set(asked.name, answers[asked.name] ?? "");
    }
    startTransition(() => dispatch(form));
  }

  return (
    <form
      noValidate
      onSubmit={(event) => {
        // Never the browser's own submission: the answers are not in the DOM
        // all at once, and the action is called from `advance`.
        event.preventDefault();
        advance();
      }}
    >
      <div className="flex flex-col gap-4">
        {questions.length > 1 ? (
          <p
            className="text-body-small text-muted-foreground"
            aria-live="polite"
          >
            Question {step + 1} of {questions.length}
          </p>
        ) : null}

        {carried === undefined ? null : (
          // The account name, for a password manager. Visually hidden and out
          // of the tab order and the accessibility tree: it is not a question
          // and a person is never asked to touch it.
          <input
            type="email"
            name="username"
            autoComplete="username"
            value={carried}
            readOnly
            tabIndex={-1}
            aria-hidden="true"
            className="sr-only"
          />
        )}

        <FormField
          label={question.label}
          htmlFor={`dashboard-step-${question.name}`}
          {...(question.hint === undefined ? {} : { hint: question.hint })}
        >
          <Input
            key={question.name}
            id={`dashboard-step-${question.name}`}
            name={question.name}
            type={question.type}
            autoComplete={question.autoComplete}
            {...(question.inputMode === undefined
              ? {}
              : { inputMode: question.inputMode })}
            {...(question.maxLength === undefined
              ? {}
              : { maxLength: question.maxLength })}
            autoFocus
            value={value}
            onChange={(event) =>
              setAnswers({ ...answers, [question.name]: event.target.value })
            }
            disabled={pending}
            aria-invalid={invalid ? "true" : undefined}
          />
        </FormField>

        <FormAlert
          error={here ? state.error : null}
          requestId={state.requestId}
        />

        <div className="flex flex-col gap-2">
          <Button type="submit" className="w-full" disabled={!valid || pending}>
            {last ? (pending ? pendingLabel : submitLabel) : "Continue"}
          </Button>
          {step === 0 ? null : (
            <Button
              type="button"
              variant="ghost"
              className="w-full"
              disabled={pending}
              onClick={() => setStep(step - 1)}
            >
              Back
            </Button>
          )}
        </div>
      </div>
    </form>
  );
}
