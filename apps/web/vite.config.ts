import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  define: {
    ...(process.env.SCREENROOM_API_ENDPOINT && !process.env.VITE_SCREENROOM_API_ENDPOINT ? { 'import.meta.env.VITE_SCREENROOM_API_ENDPOINT': JSON.stringify(process.env.SCREENROOM_API_ENDPOINT) } : {}),
    ...(process.env.SCREENROOM_USER_ID && !process.env.VITE_SCREENROOM_USER_ID ? { 'import.meta.env.VITE_SCREENROOM_USER_ID': JSON.stringify(process.env.SCREENROOM_USER_ID) } : {})
  },
  plugins: [sveltekit({ adapter: adapter({ fallback: 'index.html' }) })]
});
