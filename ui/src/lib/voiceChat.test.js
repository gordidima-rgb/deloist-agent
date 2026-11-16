import { describe, it, expect } from 'vitest';
import { splitSentences, SentenceSplitter, stripForSpeech, processVadSequence, VadStateMachine, next, createVoiceLoopState } from './voiceChat.js';

describe('splitSentences', () => {
	it('splits simple sentences', () => {
		const r = splitSentences('Привет мир. Как дела?');
		expect(r.sentences).toEqual(['Привет мир.', 'Как дела?']);
		expect(r.remainder).toBe('');
	});
	it('keeps decimal numbers', () => {
		const r = splitSentences('Цена 3.5 руб. Ок');
		expect(r.sentences).toEqual(['Цена 3.5 руб.']);
		expect(r.remainder).toBe('Ок');
	});
	it('keeps abbreviations', () => {
		const r = splitSentences('Например т.д. и т.п. Дальше');
		// abbreviation handling: should not split on т.д.
		expect(r.remainder).toContain('Дальше');
	});
	it('handles ellipsis', () => {
		const r = splitSentences('Подожди… Потом');
		expect(r.sentences.length).toBeGreaterThanOrEqual(1);
	});
	it('waits for trailing fragment', () => {
		const r = splitSentences('Готово. Остаток без точки');
		expect(r.sentences).toEqual(['Готово.']);
		expect(r.remainder).toBe('Остаток без точки');
	});
});

describe('stripForSpeech', () => {
	it('strips markdown and urls and code', () => {
		const out = stripForSpeech('**bold** и `code` и https://example.com/path');
		expect(out).not.toContain('**');
		expect(out).not.toContain('https://');
		expect(out).toContain('код');
	});
	it('replaces code blocks', () => {
		const out = stripForSpeech('текст\n```\nfoo\n```\nконец');
		expect(out).toContain('код — смотри в чате');
	});
});

describe('voice loop reducer', () => {
  it('listen -> utterance end -> recognise -> send -> speak -> turn end -> listen', () => {
    let s = createVoiceLoopState('b1');
    expect(s.phase).toBe('listening');
    let r = next(s, { type: 'utterance_end' });
    s = r.state; expect(s.phase).toBe('recognizing');
    r = next(s, { type: 'recognized', text: 'привет' });
    s = r.state; expect(s.phase).toBe('sending');
    r = next(s, { type: 'sent' });
    s = r.state; expect(s.phase).toBe('thinking');
    r = next(s, { type: 'text_delta', text: 'Готово. ' });
    s = r.state; expect(s.phase).toBe('speaking');
    r = next(s, { type: 'turn_end' });
    s = r.state; expect(s.phase).toBe('listening');
  });
  it('barge-in while speaking -> cancel + listen', () => {
    let s = createVoiceLoopState('b1');
    s = next(s, { type: 'utterance_end' }).state;
    s = next(s, { type: 'recognized', text: 'привет' }).state;
    s = next(s, { type: 'text_delta', text: 'Длинный ответ. ' }).state;
    const r = next(s, { type: 'barge_in' });
    expect(r.state.phase).toBe('listening');
    expect(r.effects).toContain('cancel');
  });
  it('approval pauses, approval_closed resumes', () => {
    let s = createVoiceLoopState('b1');
    let r = next(s, { type: 'approval' });
    expect(r.state.phase).toBe('approval');
    r = next(r.state, { type: 'approval_closed' });
    expect(r.state.phase).toBe('listening');
  });
  it('pause/resume', () => {
    let s = createVoiceLoopState('b1');
    let r = next(s, { type: 'pause' });
    expect(r.state.phase).toBe('paused');
    r = next(r.state, { type: 'resume' });
    expect(r.state.phase).toBe('listening');
  });
  it('empty recognition goes back to listening', () => {
    let s = createVoiceLoopState('b1');
    s = next(s, { type: 'utterance_end' }).state;
    const r = next(s, { type: 'recognized', text: '' });
    expect(r.state.phase).toBe('listening');
  });
});

describe('VAD', () => {
	it('start and end after silence', () => {
		const frames = [];
		// 150ms speech start (3 frames of 50ms) + silence 800ms
		for (let i = 0; i < 7; i++) frames.push(0.5);
		for (let i = 0; i < 20; i++) frames.push(0.0);
		const res = processVadSequence(frames, { frameMs: 50, threshold: 0.1 });
		expect(res.turns.length).toBe(1);
	});
	it('drops short blips', () => {
		const frames = [0.5, 0.0, 0.0, 0.0, 0.0, 0.0];
		// speech shorter than 300ms
		const res = processVadSequence(frames, { frameMs: 50, threshold: 0.1 });
		// 50ms speech < 300 -> dropped, no turn
		expect(res.turns.length).toBe(0);
	});
	it('caps at 30s', () => {
		const frames = Array(700).fill(0.5); // 35s
		const res = processVadSequence(frames, { frameMs: 50, threshold: 0.1 });
		expect(res.turns.length).toBeGreaterThanOrEqual(1);
	});
});

describe('SentenceSplitter across stream chunks', () => {
	it('keeps the space at a chunk boundary', () => {
		const s = new SentenceSplitter();
		expect(s.feed('Это голосовой ')).toEqual([]);
		expect(s.feed('разговор: без ')).toEqual([]);
		expect(s.feed('списков. Да')).toEqual(['Это голосовой разговор: без списков.']);
		expect(s.flush()).toEqual(['Да']);
	});
});
