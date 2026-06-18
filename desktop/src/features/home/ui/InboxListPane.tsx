import { Clock, ExternalLink, Mail, MailOpen, ChevronDown } from "lucide-react";
import * as React from "react";

import {
  formatInboxTypeLabel,
  type InboxFilter,
  type InboxItem,
} from "@/features/home/lib/inbox";
import { RemindersPanel } from "@/features/reminders/ui/RemindersPanel";
import { topChromeInset } from "@/shared/layout/chromeLayout";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { VirtualizedList } from "@/shared/ui/VirtualizedList";

const FILTER_OPTIONS: Array<{ label: string; value: InboxFilter }> = [
  { value: "all", label: "All" },
  { value: "mention", label: "Mentions" },
  { value: "needs_action", label: "Needs Action" },
  { value: "activity", label: "Activity" },
  { value: "agent_activity", label: "Agents" },
  { value: "reminders", label: "Reminders" },
];

type InboxListPaneProps = {
  doneSet: ReadonlySet<string>;
  filter: InboxFilter;
  items: InboxItem[];
  onFilterChange: (filter: InboxFilter) => void;
  onMarkUnread: (itemId: string) => void;
  onOpenDirect: (item: InboxItem) => void;
  onRemindLater: (item: InboxItem) => void;
  onSelect: (itemId: string) => void;
  selectedId: string | null;
  showRightDivider?: boolean;
  dueReminderCount: number;
  reminderPubkey?: string;
  activeReminderEventIds?: ReadonlySet<string>;
};

export function InboxListPane({
  doneSet,
  filter,
  items,
  onFilterChange,
  onMarkUnread,
  onOpenDirect,
  onRemindLater,
  onSelect,
  selectedId,
  showRightDivider = false,
  dueReminderCount,
  reminderPubkey,
  activeReminderEventIds,
}: InboxListPaneProps) {
  const activeFilter = FILTER_OPTIONS.find((option) => option.value === filter);
  const isReminders = filter === "reminders";
  const scrollRef = React.useRef<HTMLDivElement>(null);

  const renderItem = (item: InboxItem) => {
    const isSelected = item.id === selectedId;
    const isDone = doneSet.has(item.id);
    const hasActiveReminder = activeReminderEventIds?.has(item.id) ?? false;
    const hasChannelTarget = Boolean(item.item.channelId);
    const typeLabel = formatInboxTypeLabel(item);

    return (
      <div
        className="group/inbox-item relative"
        data-testid={`home-inbox-item-${item.id}`}
      >
        <button
          className={cn(
            "flex w-full items-start gap-2.5 border-l px-5 py-2 text-left transition-colors",
            isSelected
              ? "border-l-transparent bg-muted/30"
              : "border-l-transparent hover:bg-muted/25 active:bg-muted/40",
          )}
          onClick={() => onSelect(item.id)}
          type="button"
        >
          <div className="relative">
            <UserAvatar
              avatarUrl={item.avatarUrl}
              className="h-8 w-8"
              displayName={item.senderLabel}
              size="md"
            />
            {!isDone ? (
              <span className="absolute -right-1 -top-1 h-2.5 w-2.5 rounded-full border-2 border-background bg-primary" />
            ) : null}
          </div>

          <div className="min-w-0 flex-1">
            <div className="flex items-baseline gap-2">
              <div className="min-w-0 flex-1">
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <p className="truncate text-sm font-semibold text-foreground">
                      {item.senderLabel}
                    </p>
                    {item.isActionRequired ? (
                      <span className="inline-flex shrink-0 items-center text-2xs font-semibold uppercase tracking-[0.14em] text-amber-600 dark:text-amber-300">
                        Needs action
                      </span>
                    ) : null}
                  </div>
                  <p
                    className={cn(
                      "mt-0.5 truncate text-2xs text-muted-foreground",
                      isDone ? "font-normal" : "font-semibold",
                    )}
                  >
                    {typeLabel}
                  </p>
                </div>
              </div>
              <span
                className={cn(
                  "shrink-0 text-xs leading-5 text-muted-foreground transition-opacity group-hover/inbox-item:opacity-0 group-focus-within/inbox-item:opacity-0",
                  isDone ? "font-normal" : "font-semibold",
                )}
              >
                {item.timestampLabel}
              </span>
            </div>

            <Markdown
              className={cn(
                "mt-0.5 line-clamp-2 max-w-full text-sm! leading-5! **:inline [&_*]:text-inherit [&_a]:font-medium [&_a]:text-current [&_br]:hidden [&_p]:inline",
                isDone
                  ? "font-normal text-muted-foreground"
                  : "font-semibold text-foreground",
              )}
              content={item.preview}
              interactive={false}
              mentionNames={item.mentionNames}
              plainInlineReferences
            />
          </div>
        </button>

        <div className="absolute right-4 top-1.5 z-10 flex items-center gap-1 rounded-full border border-border/70 bg-background/95 p-0.5 opacity-0 shadow-xs backdrop-blur-sm transition-opacity group-hover/inbox-item:opacity-100 group-focus-within/inbox-item:opacity-100 supports-[backdrop-filter]:bg-background/85">
          <InboxRowActionButton
            label="Mark unread"
            onClick={() => onMarkUnread(item.id)}
          >
            <MailOpen className="h-3.5 w-3.5" />
          </InboxRowActionButton>
          <InboxRowActionButton
            disabled={!hasChannelTarget}
            label={hasChannelTarget ? "Open in channel" : "No channel link"}
            onClick={() => onOpenDirect(item)}
          >
            <ExternalLink className="h-3.5 w-3.5" />
          </InboxRowActionButton>
          <InboxRowActionButton
            active={hasActiveReminder}
            disabled={!hasChannelTarget}
            label={
              hasChannelTarget
                ? hasActiveReminder
                  ? "Reminder set"
                  : "Remind me later"
                : "Cannot remind without a channel"
            }
            onClick={() => onRemindLater(item)}
          >
            <Clock className="h-3.5 w-3.5" />
          </InboxRowActionButton>
        </div>
      </div>
    );
  };

  return (
    <section
      className={cn(
        "relative flex min-h-0 min-w-0 flex-col overflow-hidden bg-background/60",
        showRightDivider && topChromeInset.verticalDivider,
      )}
    >
      <TopChromeInsetHeader>
        <div className="px-5 py-1">
          {/* Cap to the list-column width so the right-aligned dropdown stays
              put when the pane goes full-width in reminders mode. */}
          <div className="flex min-w-0 max-w-[var(--home-inbox-list-width)] items-center justify-between gap-3">
            <h1 className="flex min-w-0 items-center gap-1.5 text-sm font-semibold leading-5 tracking-tight text-foreground">
              <Mail className="h-4 w-4 shrink-0 text-muted-foreground" />
              <span className="truncate">Inbox</span>
            </h1>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  className="inline-flex shrink-0 items-center gap-1.5 rounded-full border-border/70 bg-background/70 px-2.5 text-2xs font-medium leading-none text-muted-foreground shadow-xs backdrop-blur-sm hover:bg-muted/60 hover:text-foreground"
                  data-testid="inbox-filter-trigger"
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  <span>{activeFilter?.label ?? "All"}</span>
                  {dueReminderCount > 0 ? (
                    <span
                      className="inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-2xs font-semibold leading-none text-primary-foreground"
                      data-testid="inbox-reminder-badge"
                    >
                      {dueReminderCount}
                    </span>
                  ) : null}
                  <ChevronDown className="h-4 w-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="min-w-40">
                <DropdownMenuRadioGroup
                  onValueChange={(value) =>
                    onFilterChange(value as InboxFilter)
                  }
                  value={filter}
                >
                  {FILTER_OPTIONS.map((option) => (
                    <DropdownMenuRadioItem
                      key={option.value}
                      value={option.value}
                    >
                      <span className="flex flex-1 items-center justify-between gap-2">
                        {option.label}
                        {option.value === "reminders" &&
                        dueReminderCount > 0 ? (
                          <span
                            className="inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-2xs font-semibold leading-none text-primary-foreground"
                            data-testid="inbox-reminder-badge-option"
                          >
                            {dueReminderCount}
                          </span>
                        ) : null}
                      </span>
                    </DropdownMenuRadioItem>
                  ))}
                </DropdownMenuRadioGroup>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </div>
      </TopChromeInsetHeader>

      {isReminders ? (
        <div
          className="flex min-h-0 flex-1 flex-col overflow-hidden"
          data-testid="home-inbox-reminders"
        >
          {reminderPubkey ? (
            <RemindersPanel includeDone pubkey={reminderPubkey} />
          ) : null}
        </div>
      ) : (
        <div
          className="min-h-0 flex-1 overflow-y-auto overscroll-contain"
          data-testid="home-inbox-list"
          ref={scrollRef}
        >
          {items.length === 0 ? (
            <div className="flex h-full min-h-64 items-center justify-center px-6 text-center">
              <div>
                <p className="text-sm font-medium text-foreground">
                  No messages found
                </p>
                <p className="mt-1 text-sm text-muted-foreground">
                  Switch back to all mail to see more messages.
                </p>
              </div>
            </div>
          ) : (
            <VirtualizedList
              estimateSize={76}
              getItemKey={(item) => item.id}
              items={items}
              renderItem={renderItem}
              scrollRef={scrollRef}
            />
          )}
        </div>
      )}
    </section>
  );
}

function InboxRowActionButton({
  active = false,
  children,
  disabled = false,
  label,
  onClick,
}: {
  active?: boolean;
  children: React.ReactNode;
  disabled?: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          aria-label={label}
          disabled={disabled}
          className={cn(
            "flex h-7 w-7 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-40",
            active && "bg-blue-500/10 text-blue-500 hover:text-blue-500",
          )}
          onClick={(event) => {
            event.preventDefault();
            event.stopPropagation();
            if (disabled) {
              return;
            }
            onClick();
          }}
          type="button"
        >
          {children}
        </button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}
