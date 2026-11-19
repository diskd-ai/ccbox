import { ReactNode } from "react";
import { Link } from "react-router-dom";
import { ArrowRight } from "lucide-react";

interface SummaryCardProps {
  title: string;
  count: number;
  viewAllPath: string;
  children: ReactNode;
}

export function SummaryCard({ title, count, viewAllPath, children }: SummaryCardProps) {
  return (
    <div className="border border-border bg-surface rounded-xl shadow-card flex flex-col overflow-hidden">
      <div className="flex items-center justify-between px-4 py-2.5 border-b border-border">
        <h3 className="text-sm font-sans font-semibold text-foreground tracking-wide uppercase">
          {title}
        </h3>
        <span className="text-xs text-text-muted font-medium">({count})</span>
      </div>
      <div className="flex-1 px-4 py-2 space-y-0.5 text-sm">
        {children}
      </div>
      <Link
        to={viewAllPath}
        className="flex items-center gap-1.5 px-4 py-2.5 text-xs font-medium text-text-secondary hover:text-foreground hover:bg-bg-hover border-t border-border transition-colors"
      >
        View all <ArrowRight className="w-3 h-3" />
      </Link>
    </div>
  );
}
