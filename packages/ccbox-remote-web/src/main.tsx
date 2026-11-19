import { createRoot } from "react-dom/client";
import App from "./App.tsx";
import "./index.css";
import { usePreferencesStore } from "./stores/preferencesStore";

// Apply theme before first render to avoid flash
usePreferencesStore.getState().initTheme();

createRoot(document.getElementById("root")!).render(<App />);
