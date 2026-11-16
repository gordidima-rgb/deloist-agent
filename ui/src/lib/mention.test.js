import { describe, it, expect } from 'vitest';
import { detectMentionTrigger } from './mention.js';

describe('detectMentionTrigger', () => {
	describe('@ trigger (lenient)', () => {
		it('triggers on bare @ (opens full list)', () => {
			expect(detectMentionTrigger('@')).toEqual({ trigger: '@', query: '' });
		});
		it('triggers on @ after whitespace', () => {
			expect(detectMentionTrigger('купить хлеб @')).toEqual({ trigger: '@', query: '' });
		});
		it('captures the query after @', () => {
			expect(detectMentionTrigger('@Дм')).toEqual({ trigger: '@', query: 'Дм' });
		});
		it('does not trigger when @ is mid-word (no boundary)', () => {
			expect(detectMentionTrigger('mail@dom')).toBeNull();
		});
	});

	it('returns null for empty text', () => {
		expect(detectMentionTrigger('')).toBeNull();
	});
	it('returns null for plain text with no trigger', () => {
		expect(detectMentionTrigger('просто текст задачи')).toBeNull();
	});
});
