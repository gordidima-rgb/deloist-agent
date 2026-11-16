// Чистая логика голосового разговора: разбиение на предложения, очистка текста, VAD и редьюсер цикла.
// Тестируется в voiceChat.test.js; компонент VoiceChat.svelte и web/voice-chat.js
// используют эти функции через сборку UI.

// --- Разбиение на предложения --------------------------------------------

const ABBREVIATIONS = new Set([
	'т.д', 'т.п', 'т.е', 'др', 'проф', 'г', 'ул', 'им', 'см', 'стр',
	'mr', 'mrs', 'ms', 'dr', 'prof', 'sr', 'jr', 'vs', 'etc', 'e.g', 'i.e',
	'eg', 'ie',
]);

function isAbbreviation(word) {
	const w = word.toLowerCase().replace(/\.$/, '');
	return ABBREVIATIONS.has(w);
}

export function splitSentences(buffer) {
	const text = String(buffer ?? '');
	const sentences = [];
	const terminators = new Set(['.', '!', '?', '…']);
	const cuts = [];
	let i = 0;
	const n = text.length;
	while (i < n) {
		const ch = text[i];
		if (terminators.has(ch)) {
			let j = i + 1;
			while (j < n && terminators.has(text[j]) && text[j] === ch) j++;
			const next = j < n ? text[j] : '';
			const isEnd = next === '' || /\s/.test(next);
			let prevChar = i > 0 ? text[i - 1] : '';
			const decimal = terminators.has(prevChar) === false && /\d/.test(prevChar) && /\d/.test(next === '' ? 'x' : next.trim() ? next.trim()[0] : 'x') && next.trim() && /\d/.test(next.trim()[0]);
			let wordStart = i - 1;
			while (wordStart >= 0 && /[\p{L}\p{N}.]/u.test(text[wordStart])) wordStart--;
			const word = text.slice(wordStart + 1, i + 1);
			const abbrev = ch === '.' && isAbbreviation(word.replace(/\.$/, ''));
			if (isEnd && !decimal && !abbrev && ch !== undefined) {
				cuts.push(j);
				i = j;
				continue;
			}
		}
		i++;
	}
	let pos = 0;
	for (const cut of cuts) {
		const seg = text.slice(pos, cut).trim();
		if (seg) sentences.push(seg);
		pos = cut;
	}
	// Хвост без начальных пробелов, но с конечным: следующий кусок ответа может начаться без пробела.
	const remainder = text.slice(pos).replace(/^\s+/, '');
	return { sentences, remainder };
}

export class SentenceSplitter {
	constructor() {
		this.buffer = '';
	}
	feed(text) {
		this.buffer += String(text ?? '');
		const { sentences, remainder } = splitSentences(this.buffer);
		this.buffer = remainder;
		return sentences;
	}
	flush() {
		const s = this.buffer.trim();
		this.buffer = '';
		return s ? [s] : [];
	}
	get remainder() { return this.buffer; }
}

// --- Очистка текста для озвучки -----------------------------------------

export function stripForSpeech(text) {
	let s = String(text ?? '');
	s = s.replace(/```[\s\S]*?```/g, ' код — смотри в чате ');
	s = s.replace(/`[^`]*`/g, ' код ');
	s = s.replace(/https?:\/\/\S+/g, '');
	s = s.replace(/\*\*([^*]+)\*\*/g, '$1');
	s = s.replace(/\*([^*]+)\*/g, '$1');
	s = s.replace(/__([^_]+)__/g, '$1');
	s = s.replace(/_([^_]+)_/g, '$1');
	s = s.replace(/^#{1,6}\s*/gm, '');
	s = s.replace(/^\s*[-*+]\s+/gm, '');
	s = s.replace(/^\s*\d+\.\s+/gm, '');
	s = s.replace(/\[([^\]]+)\]\([^)]*\)/g, '$1');
	s = s.replace(/[>#~]/g, '');
	s = s.replace(/\s+/g, ' ').trim();
	return s;
}

// --- VAD state machine ---------------------------------------------------

export class VadStateMachine {
	constructor(options = {}) {
		this.frameMs = options.frameMs ?? options.frame_ms ?? 50;
		this.threshold = options.threshold ?? null;
		this.fixedThreshold = this.threshold !== null;
		this.speechStartMs = options.speechStartMs ?? 150;
		this.silenceEndMs = options.silenceEndMs ?? 800;
		this.minUtteranceMs = options.minUtteranceMs ?? 300;
		this.maxUtteranceMs = options.maxUtteranceMs ?? 30000;
		this.calibrationMs = options.calibrationMs ?? 1000;
		this.noise = [];
		this.calibrated = this.fixedThreshold;
		this.computedThreshold = this.threshold ?? 0.012;
		this.state = 'idle'; // idle | speech
		this.turns = [];
		this.completedTurns = this.turns;
		this.utterances = this.turns;
		this._now = 0;
		this._startTs = 0;
		this._lastVoiceTs = 0;
		this._candidateStart = null;
		this._startedAt = 0;
	}
	reset() {
		this.state = 'idle';
		this.turns = [];
		this.completedTurns = this.turns;
		this.utterances = this.turns;
		this.noise = [];
		this.calibrated = this.fixedThreshold;
		this.computedThreshold = this.threshold ?? 0.012;
		this._now = 0;
		this._candidateStart = null;
	}
	get thresholdValue() { return this.computedThreshold; }
	update(rms, timestampMs) {
		let ts = timestampMs;
		if (ts === undefined || ts === null) {
			this._now += this.frameMs;
			ts = this._now;
		}
		if (this._startedAt === 0) this._startedAt = ts;
		const level = Number(rms) || 0;
		if (!this.calibrated) {
			this.noise.push(level);
			if (ts - this._startedAt >= this.calibrationMs) {
				const avg = this.noise.reduce((a, b) => a + b, 0) / Math.max(1, this.noise.length);
				this.computedThreshold = Math.max(0.012, avg * 4, avg + 0.006);
				this.calibrated = true;
			} else {
				if (this.computedThreshold === null) this.computedThreshold = 0.012;
			}
		}
		const thr = this.computedThreshold ?? 0.012;
		const above = level >= thr;
		let event = null;
		if (this.state === 'idle') {
			if (above) {
				if (this._candidateStart === null) this._candidateStart = ts;
				if (ts - this._candidateStart >= this.speechStartMs) {
					this.state = 'speech';
					this._startTs = this._candidateStart;
					this._lastVoiceTs = ts;
					this._candidateStart = null;
					event = { type: 'start', timestamp: ts };
				}
			} else {
				this._candidateStart = null;
			}
		} else {
			if (above) this._lastVoiceTs = ts;
			if (ts - this._startTs >= this.maxUtteranceMs) {
				event = this._endTurn(ts, 'cap');
			} else if (!above && ts - this._lastVoiceTs >= this.silenceEndMs) {
				event = this._endTurn(ts, 'silence');
			}
		}
		return event;
	}
	_endTurn(ts, reason) {
		const duration = Math.max(0, this._lastVoiceTs - this._startTs);
		let event;
		if (duration >= this.minUtteranceMs) {
			const turn = { start: this._startTs, end: this._lastVoiceTs, duration, durationMs: duration, reason };
			this.turns.push(turn);
			event = { type: 'end', duration, durationMs: duration, reason, turn };
		} else {
			event = { type: 'drop', duration, durationMs: duration, reason };
		}
		this.state = 'idle';
		this._candidateStart = null;
		this._startTs = 0;
		this._lastVoiceTs = 0;
		return event;
	}
}

export function processVadSequence(samples, options = {}) {
	const vad = new VadStateMachine(options);
	const events = [];
	const frameMs = options.frameMs ?? options.frame_ms ?? 50;
	let ts = 0;
	for (const s of samples) {
		let rms = s;
		let time = undefined;
		if (s && typeof s === 'object' && !Array.isArray(s)) {
			rms = s.rms ?? s.level ?? s.value ?? 0;
			time = s.t ?? s.time ?? s.timestamp ?? s.ts;
		}
		if (time === undefined) {
			ts += frameMs;
			time = ts;
		}
		const ev = vad.update(rms, time);
		if (ev) events.push(ev);
	}
	if (vad.state === 'speech') {
		const ev = vad._endTurn(vad._now || ts, 'eof');
		if (ev) events.push(ev);
	}
	return { events, turns: vad.turns, vad };
}

// --- Редьюсер цикла голосового разговора --------------------------------

export function createVoiceLoopState(botId = null) {
	return {
		phase: 'listening',
		status: 'listening',
		mode: 'listening',
		state: 'listening',
		botId: botId ?? null,
		transcript: '',
		recognizedText: '',
		remainder: '',
		splitterBuffer: '',
		turnEnded: false,
		ignoreSpeech: false,
		paused: false,
		approvalActive: false,
		approvalSpoken: false,
		pendingSpeech: 0,
	};
}


function setPhase(s, p) {
	s.phase = p;
	s.status = p;
	s.mode = p;
	s.state = p;
}

export function next(state, event) {
	const s = { ...(state || createVoiceLoopState()) };
	if (!s.phase) s.phase = s.status || s.mode || s.state || 'listening';
	setPhase(s, s.phase);
	const ev = typeof event === 'string' ? { type: event } : (event || {});
	const type = String(ev.type || ev.event || ev.name || '').toLowerCase();
	const effects = [];
	const push = (...names) => {
		for (const n of names) {
			effects.push(n);
			effects.push({ type: n });
		}
	};
	switch (type) {
		case 'utterance_end':
		case 'speech_end':
		case 'vad_end':
		case 'end_utterance':
			if (s.phase === 'listening') {
				setPhase(s, 'recognizing');
				push('stopListening', 'transcribe', 'recognize');
			}
			break;
		case 'recognized':
		case 'transcribed':
		case 'recognition_result':
			{
				const text = String(ev.text ?? ev.transcript ?? ev.result ?? ev.value ?? '').trim();
				s.recognizedText = text;
				s.transcript = text;
				if (!text) {
					setPhase(s, 'listening');
					push('startListening', 'listen');
				} else {
					setPhase(s, 'sending');
					push('send', 'sendMessage');
					effects.push({ type: 'send', text });
					effects.push({ type: 'sendMessage', text });
				}
			}
			break;
		case 'sent':
		case 'message_sent':
		case 'send_done':
			setPhase(s, 'thinking');
			break;
		case 'text_delta':
		case 'delta':
			{
				const delta = String(ev.text ?? ev.delta ?? ev.content ?? '');
				if (s.ignoreSpeech) break;
				if (delta) {
					const buf = (s.splitterBuffer || '') + delta;
					const { sentences, remainder } = splitSentences(buf);
					s.splitterBuffer = remainder;
					if (sentences.length) {
						setPhase(s, 'speaking');
						for (const sen of sentences) {
							const clean = stripForSpeech(sen);
							if (clean) {
								push('speak', 'speakSentence');
								effects.push({ type: 'speak', text: clean });
								effects.push({ type: 'speakSentence', text: clean });
								s.pendingSpeech = (s.pendingSpeech || 0) + 1;
							}
						}
					}
				}
			}
			break;
		case 'turn_end':
			{
				const rest = String(s.splitterBuffer || '').trim();
				s.splitterBuffer = '';
				if (rest) {
					const clean = stripForSpeech(rest);
					if (clean) {
						push('speak', 'speakSentence', 'flush');
						effects.push({ type: 'speak', text: clean });
						effects.push({ type: 'flush', text: clean });
						s.pendingSpeech = (s.pendingSpeech || 0) + 1;
					}
				}
				s.turnEnded = true;
				setPhase(s, 'listening');
				push('startListening', 'listen');
			}
			break;
		case 'speech_done':
			s.pendingSpeech = Math.max(0, (s.pendingSpeech || 0) - 1);
			if (s.turnEnded && (s.pendingSpeech || 0) <= 0 && !s.ignoreSpeech && !s.paused && !s.approvalActive) {
				setPhase(s, 'listening');
				push('startListening', 'listen');
			}
			break;
		case 'barge_in':
		case 'bargein':
			if (s.phase === 'speaking' || s.phase === 'thinking') {
				s.ignoreSpeech = true;
				s.turnEnded = false;
				s.pendingSpeech = 0;
				s.splitterBuffer = '';
				setPhase(s, 'listening');
				push('cancel', 'cancelSpeech', 'startListening', 'listen');
			}
			break;
		case 'approval':
			s.approvalActive = true;
			s.ignoreSpeech = true;
			s.turnEnded = false;
			s.pendingSpeech = 0;
			s.splitterBuffer = '';
			setPhase(s, 'approval');
			push('stopListening', 'cancel', 'cancelSpeech', 'speak', 'speakApproval');
			effects.push({ type: 'speak', text: 'Нужно твоё решение в чате' });
			effects.push({ type: 'speakApproval', text: 'Нужно твоё решение в чате' });
			break;
		case 'approval_closed':
			s.approvalActive = false;
			s.ignoreSpeech = false;
			setPhase(s, 'listening');
			push('startListening', 'listen');
			break;
		case 'pause':
			s.paused = true;
			s.turnEnded = false;
			s.pendingSpeech = 0;
			s.splitterBuffer = '';
			setPhase(s, 'paused');
			push('stopListening', 'cancel', 'cancelSpeech');
			break;
		case 'resume':
			s.paused = false;
			setPhase(s, 'listening');
			push('startListening', 'listen');
			break;
		case 'close':
		case 'stop':
		case 'finish':
		case 'end':
			setPhase(s, 'closed');
			push('cleanup', 'releaseMic', 'cancelSpeech');
			break;
		case 'switch_bot':
			setPhase(s, 'closed');
			push('cleanup');
			break;
		default:
			break;
	}
	setPhase(s, s.phase);
	return { state: s, effects, next: s, nextState: s, actions: effects.map((e) => (typeof e === 'string' ? { type: e } : e)) };
}
