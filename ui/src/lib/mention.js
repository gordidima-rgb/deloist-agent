// Detection of the @-mention trigger in chat input.
//
// Only the "@" trigger is supported here (the "+" trigger from the reference
// implementation is intentionally dropped).

/**
 * @typedef {{ trigger: '@', query: string }} MentionTrigger
 */

// `(^|\s)` — word boundary; `@` — trigger char; `[\p{L}\p{N}_]*` — query up to the caret.
const TRIGGER_RE = /(^|\s)(@)([\p{L}\p{N}_]*)$/u;

/**
 * Inspect the text up to the caret and decide whether a mention dropdown should open.
 * Returns the trigger char and the query typed after it, or null if nothing should trigger.
 * @param {string} textBefore
 * @returns {MentionTrigger | null}
 */
export function detectMentionTrigger(textBefore) {
	const m = textBefore.match(TRIGGER_RE);
	if (!m) return null;
	return { trigger: '@', query: m[3] };
}
