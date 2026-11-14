import React from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';

const storedTheme = typeof window !== 'undefined' ? window.localStorage.getItem('sweeper-theme') : null;
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
