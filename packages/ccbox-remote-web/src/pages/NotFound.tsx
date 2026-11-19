import { useLocation, Link } from "react-router-dom";
import { useEffect } from "react";
import { ArrowLeft } from "lucide-react";

const NotFound = () => {
  const location = useLocation();

  useEffect(() => {
    console.error("404 Error: User attempted to access non-existent route:", location.pathname);
  }, [location.pathname]);

  return (
    <div className="flex items-center justify-center h-full p-8">
      <div className="text-center space-y-4 max-w-sm">
        <div className="w-16 h-16 rounded-2xl bg-muted flex items-center justify-center mx-auto shadow-card">
          <span className="text-2xl font-mono font-bold text-text-muted">404</span>
        </div>
        <h1 className="text-xl font-sans font-semibold text-foreground">Page Not Found</h1>
        <p className="text-sm text-text-secondary">
          The path <code className="font-mono text-foreground bg-secondary px-1.5 py-0.5 rounded-md">{location.pathname}</code> doesn't exist.
        </p>
        <Link
          to="/"
          className="inline-flex items-center gap-2 px-4 py-2 text-sm font-sans font-medium bg-primary text-primary-foreground rounded-lg hover:opacity-90 transition-opacity shadow-card"
        >
          <ArrowLeft className="w-3.5 h-3.5" />
          Back to Dashboard
        </Link>
      </div>
    </div>
  );
};

export default NotFound;
