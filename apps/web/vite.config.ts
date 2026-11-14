import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';
import { operatorBrowserConfig, operatorProxyPlugin, readOperatorConfig } from './server/operator-proxy.mjs';

const operator = readOperatorConfig();

export default defineConfig({
  define: {
    __SCREENROOM_WEB_OPERATOR__: JSON.stringify(operatorBrowserConfig(operator)),
    ...(process.env.SCREENROOM_API_ENDPOINT && !process.env.VITE_SCREENROOM_API_ENDPOINT ? { 'import.meta.env.VITE_SCREENROOM_API_ENDPOINT': JSON.stringify(process.env.SCREENROOM_API_ENDPOINT) } : {}),
    ...(process.env.SCREENROOM_USER_ID && !process.env.VITE_SCREENROOM_USER_ID ? { 'import.meta.env.VITE_SCREENROOM_USER_ID': JSON.stringify(process.env.SCREENROOM_USER_ID) } : {})
  },
  plugins: [operatorProxyPlugin(operator), sveltekit({ adapter: adapter({ fallback: 'index.html' }) })]
});
