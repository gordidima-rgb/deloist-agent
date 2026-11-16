// Detection of the @- and +-mention trigger in chat and planner input.
//
// "@Имя" and "+Имя" both pick a bot; the sign typed stays in the text.

/**
 * @typedef {{ trigger: '@' | '+', query: string }} MentionTrigger
 */

// `(^|\s)` — word boundary; `[@+]` — trigger char; `[\p{L}\p{N}_]*` — query up to the caret.
export const TRIGGER_RE = /(^|\s)([@+])([\p{L}\p{N}_]*)$/u;

/**
 * Inspect the text up to the caret and decide whether a mention dropdown should open.
 * Returns the trigger char and the query typed after it, or null if nothing should trigger.
 * @param {string} textBefore
 * @returns {MentionTrigger | null}
 */
export function detectMentionTrigger(textBefore) {
	const m = textBefore.match(TRIGGER_RE);
	if (!m) return null;
	return { trigger: /** @type {'@' | '+'} */ (m[2]), query: m[3] };
}
