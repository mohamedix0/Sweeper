import React from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';

let storedTheme: string | null = null;
if (typeof window !== 'undefined') {
  try {
    storedTheme = window.localStorage.getItem('sweeper-theme');
  } catch {
    storedTheme = null;
  }
}
const initialTheme = storedTheme === 'light' ? 'light' : 'dark';
document.documentElement.dataset.theme = initialTheme;

const rootEl = document.getElementById('root');
if (!rootEl) {
  throw new Error('Root element #root not found');
}

createRoot(rootEl).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
