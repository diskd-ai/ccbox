import { motion, AnimatePresence } from "framer-motion";
import { X } from "lucide-react";
import { ALL_SHORTCUT_GROUPS } from "@/hooks/useKeyboardShortcuts";

interface KeyboardShortcutsHelpProps {
  open: boolean;
  onClose: () => void;
}

export function KeyboardShortcutsHelp({ open, onClose }: KeyboardShortcutsHelpProps) {
  return (
    <AnimatePresence>
      {open && (
        <>
          <motion.div
            className="fixed inset-0 bg-background/60 backdrop-blur-sm z-50"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={onClose}
          />
          <motion.div
            className="fixed inset-4 md:inset-auto md:top-[10%] md:left-1/2 md:-translate-x-1/2 md:w-full md:max-w-2xl md:max-h-[80vh] z-50 border border-border bg-surface rounded-xl shadow-dropdown overflow-y-auto"
            initial={{ opacity: 0, scale: 0.97 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.97 }}
            transition={{ duration: 0.15 }}
          >
            <div className="flex items-center justify-between px-4 py-3 border-b border-border sticky top-0 bg-surface rounded-t-xl">
              <h2 className="text-lg font-sans font-semibold text-foreground">Keyboard Shortcuts</h2>
              <button onClick={onClose} className="text-text-secondary hover:text-foreground transition-colors p-1">
                <X className="w-4 h-4" />
              </button>
            </div>

            <div className="p-4 grid grid-cols-1 md:grid-cols-2 gap-6">
              {ALL_SHORTCUT_GROUPS.map((group) => (
                <div key={group.title}>
                  <h3 className="text-xs font-sans font-medium text-text-secondary uppercase tracking-wider mb-2">
                    {group.title}
                  </h3>
                  <div className="space-y-1">
                    {group.shortcuts.map((sc) => (
                      <div key={sc.keys} className="flex items-center justify-between py-1.5">
                        <span className="text-sm text-foreground">{sc.description}</span>
                        <kbd className="text-xs text-text-secondary bg-secondary px-2 py-0.5 font-mono ml-4 shrink-0 rounded-md">
                          {sc.keys}
                        </kbd>
                      </div>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}
