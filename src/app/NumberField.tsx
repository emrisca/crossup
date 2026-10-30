/**
 * A number you type, with no stepper.
 *
 * A `number` input carries spin buttons that sit exactly where the cursor lands
 * when you aim at the field, and one click of them moves the value by one —
 * useless for a duration set in hundreds of milliseconds, and easy to hit by
 * accident. This is a plain text box that keeps the digits and drops everything
 * else, so retyping over a value, or pasting `1 500 ms` into it, lands on 1500
 * rather than refusing the paste or emptying the field.
 *
 * It is allowed to be empty while a new number is being typed: an empty box
 * stores nothing, so the value it held is still there if you change your mind,
 * and it comes back on the way out.
 *
 * `signed` allows a leading minus, which is what screen coordinates need: a
 * second monitor to the left of the primary one is at a negative x, and a point
 * above the top of the desktop is at a negative y. The minus is lifted to the
 * front wherever it is typed, so `5-` and `-5` both end up as -5.
 */

import { useState } from "react";

export function NumberField({
  label,
  value,
  title,
  signed = false,
  onValueChange,
}: {
  label: string;
  value: number;
  title?: string;
  /** Accept a leading minus, for coordinates. */
  signed?: boolean;
  onValueChange: (value: number) => void;
}) {
  // What has been typed, while it is out of step with the stored number. Null
  // means the box is showing the value it stands for.
  const [typed, setTyped] = useState<string | null>(null);

  const retype = (raw: string) => {
    const digits = signed
      ? // One minus, moved to the front, however it was typed.
        (raw.includes("-") ? "-" : "") + raw.replace(/\D/g, "")
      : raw.replace(/\D/g, "");
    setTyped(digits);
    // A lone minus is a number on its way in, not a value to store.
    if (digits !== "" && digits !== "-") onValueChange(Number(digits));
  };

  return (
    <input
      className="hb__field hb__num"
      type="text"
      inputMode={signed ? "text" : "numeric"}
      autoComplete="off"
      spellCheck={false}
      aria-label={label}
      title={title}
      value={typed ?? String(value)}
      onChange={(event) => retype(event.target.value)}
      onBlur={() => setTyped(null)}
    />
  );
}
