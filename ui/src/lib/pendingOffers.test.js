import { describe, expect, it } from 'vitest';
import { offerEventsToShow } from './pendingOffers.js';

const team = { type: 'team_proposal', botId: 'b', proposal: { id: 'offer_1', kind: 'team', newBots: [] } };
const change = { type: 'team_proposal', botId: 'b', proposal: { id: 'offer_2', kind: 'bot_change' } };
const specialist = { type: 'specialist_offer', botId: 'b', offer: { id: 'offer_3', specialistName: 'Ин' } };

describe('offerEventsToShow', () => {
  it('turns the server list into events for cards that are not on the page yet', () => {
    const events = offerEventsToShow('b', [team, change, specialist], () => false);
    expect(events.map((e) => e.type)).toEqual(['team_proposal', 'team_proposal', 'specialist_offer']);
    expect(events.every((e) => e.botId === 'b')).toBe(true);
  });

  it('skips cards that are already shown and repeats of the same card', () => {
    const events = offerEventsToShow('b', [team, change, team], (id) => id === 'offer_2');
    expect(events.map((e) => e.proposal.id)).toEqual(['offer_1']);
  });

  it('ignores junk and an empty answer from an old server', () => {
    expect(offerEventsToShow('b', undefined, () => false)).toEqual([]);
    expect(offerEventsToShow('b', [null, 5, { type: 'x' }, { type: 'team_proposal' }], () => false)).toEqual([]);
  });
});
