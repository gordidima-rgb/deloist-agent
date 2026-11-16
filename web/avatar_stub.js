// Public build: the glass-orb avatars are not part of this repository.
// The page falls back to a coloured circle with the bot's initial.
(() => {
  window.CrewAvatar = {
    ORBS: [],
    COLORS: ['#ff6a4d', '#f2a93b', '#a3c14a', '#4fbf8a', '#3fb8bd', '#4f9ce6', '#3d6cf0', '#6f6fe0', '#8e6ac9', '#d06ac0', '#e8728f', '#7a8595'],
    pointAt() {},
    mount() { return null; },
    stats() { return { frames: 0, avgMs: 0, avatars: 0 }; },
  };
})();
