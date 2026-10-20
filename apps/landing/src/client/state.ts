// Page-level flags shared by the controllers (keyboard priority, first scroll).
export const page = {
  overlay: false,
  inboxLive: false,
  inboxFocused: false,
  armed: false,
  onArm: [] as (() => void)[],
};
