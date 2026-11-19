import { motion, AnimatePresence } from "framer-motion";
import { WifiOff, Loader2 } from "lucide-react";
import { useConnection } from "@/hooks/useConnection";

export function ConnectionOverlay() {
  const { state, reconnectAttempt, error } = useConnection();

  const showOverlay = state === "reconnecting" || state === "auth_failed";

  return (
    <AnimatePresence>
      {showOverlay && (
        <motion.div
          className="fixed inset-0 z-40 bg-background/90 backdrop-blur-sm flex items-center justify-center"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
        >
          <div className="bg-surface border border-border rounded-xl shadow-dropdown p-6 max-w-sm mx-4 text-center space-y-4">
            {state === "reconnecting" ? (
              <>
                <div className="w-14 h-14 rounded-2xl bg-status-warning/10 flex items-center justify-center mx-auto">
                  <Loader2 className="w-7 h-7 text-status-warning animate-spin" />
                </div>
                <h2 className="text-lg font-sans font-semibold text-foreground">
                  Reconnecting...
                </h2>
                <p className="text-sm text-text-secondary">
                  Attempt {reconnectAttempt}. Connection will be restored automatically.
                </p>
              </>
            ) : (
              <>
                <div className="w-14 h-14 rounded-2xl bg-status-error/10 flex items-center justify-center mx-auto">
                  <WifiOff className="w-7 h-7 text-status-error" />
                </div>
                <h2 className="text-lg font-sans font-semibold text-foreground">
                  Authentication Failed
                </h2>
                <p className="text-sm text-text-secondary">
                  {error || "Could not authenticate with CCBox."}
                </p>
              </>
            )}
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
