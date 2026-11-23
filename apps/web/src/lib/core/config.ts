declare global {
  interface Window {
    __SCREENROOM_USER_ID__?: string;
  }
}

export function webConfig() {
  const selector = typeof window === 'undefined' ? undefined : window.__SCREENROOM_USER_ID__;
  if (typeof window !== 'undefined' && (selector !== undefined || import.meta.env.VITE_SCREENROOM_SAME_ORIGIN === '1')) {
    return { endpoint: window.location.origin, user_id: selector ?? '' };
  }
  return {
    endpoint: import.meta.env.VITE_SCREENROOM_API_ENDPOINT || 'https://preview-screenroom.holdenitdown.net',
    user_id: import.meta.env.VITE_SCREENROOM_USER_ID || ''
  };
}
