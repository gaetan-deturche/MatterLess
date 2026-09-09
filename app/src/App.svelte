<script lang="ts">
  import * as api from "./lib/api";
  import * as store from "./lib/store.svelte";
  import * as log from "./lib/log";
  import * as toast from "./lib/toast";
  import MessageList from "./lib/MessageList.svelte";
  import ThreadPane from "./lib/ThreadPane.svelte";
  import ChannelIcon from "./lib/ChannelIcon.svelte";
  import Avatar from "./lib/Avatar.svelte";
  import * as media from "./lib/media";
  import * as virtual from "./lib/virtual";
  import { listen } from "@tauri-apps/api/event";
  import { tick } from "svelte";
  import Composer from "./lib/Composer.svelte";
  import Switcher from "./lib/Switcher.svelte";
  import Search from "./lib/Search.svelte";
  import Kept from "./lib/Kept.svelte";
  import BrowseChannels from "./lib/BrowseChannels.svelte";
  import NewConversation from "./lib/NewConversation.svelte";
  import ChannelMenu from "./lib/ChannelMenu.svelte";
  import AddMembers from "./lib/AddMembers.svelte";
  import ProfileCard from "./lib/ProfileCard.svelte";
  import Scrollbar from "./lib/Scrollbar.svelte";

  let signedIn = $state<boolean | undefined>(undefined);
  let loginId = $state("");
  let password = $state("");
  let mfaToken = $state("");
  /** The server asked for a second factor, so the code screen replaces the
   *  credentials one. Only ever set by a sign-in that got that far. */
  let mfaNeeded = $state(false);

  /** The channel a right-click opened a menu on, and where the pointer was. */
  let channelMenu: { channel: api.ChannelSummary; x: number; y: number } | null = $state(null);
  /** The channel whose member picker is open. */
  let addingTo: api.ChannelSummary | null = $state(null);
  let error = $state("");
  let busy = $state(false);

  // Views are computed over the store, never a module-level $derived gated on a
  // nullable: an early return before any reactive read memoises empty forever.
  const channels = $derived(store.channelList() ?? []);
  const active = $derived(store.activeChannel());
  const rows = $derived(active ? store.rowsOf(active) : undefined);
  const openThread = $derived(store.activeThread());

  // ---- the sidebar, grouped as the reader arranged it server-side --------
  //
  // Two levels, because the server has two: a team
  // holds categories (`Favorites`, a custom `Horde`, `Channels`), and direct
  // messages are merged into one group of their own -- the DM category is
  // returned once per team with the same conversations in it.
  const groups = $derived(store.sidebarGroups());

  /** The groups as drawn, with unread channels lifted into their own leading
   *  group when that is how the reader reads.
   *
   *  Derived rather than fetched: which channels are unread changes with every
   *  arriving message and every channel read, and computing the split in Rust
   *  meant a channel stayed under "Unreads" after it had been read -- until
   *  something unrelated triggered a regroup. Here it follows the store.
   */
  const arranged = $derived.by(() => {
    const all = groups ?? [];
    if (!store.separateUnreads()) return all;

    const unread: api.ChannelSummary[] = [];
    const rest = all.map((group) => {
      const staying = group.channels.filter((channel) => {
        const state = store.unreadOf(channel.id);
        // Muted channels stay put however much they hold: muting says "do not
        // interrupt me".
        const asking = (state?.messages ?? 0) > 0 && !state?.muted;
        if (asking) unread.push(channel);
        return !asking;
      });
      return { ...group, channels: staying };
    });

    // A category emptied by the move has nothing left to say.
    const kept = rest.filter((group) => group.channels.length > 0);
    if (unread.length === 0) return kept;

    unread.sort((left, right) => right.last_post_at - left.last_post_at);
    return [
      {
        id: "unreads",
        display_name: "Unreads",
        sort_order: -1,
        category_type: "unreads",
        team_id: "",
        team_name: "",
        collapsed: false,
        channels: unread,
      },
      ...kept,
    ];
  });

  /** The display-options panel in the channel header. */
  let optionsOpen = $state(false);
  /** The quick switcher, on Ctrl+K. */
  let switcherOpen = $state(false);
  /** The search pane, from the header field or Ctrl+Shift+F. */
  /** Where the reader has been, in order, and where they are in it.
   *
   *  A place is a channel plus the thread open over it, because those are the
   *  two things a "back" is asking to undo -- opening a thread and then going
   *  back should close it rather than leave the channel.
   *
   *  Recorded by watching where the app *is*, not by instrumenting the ways of
   *  getting there: the sidebar, the switcher, a search result, a permalink, a
   *  mention and "message this person" all arrive at a place, and every one of
   *  them would otherwise have to remember to say so. */
  type Place = { channelId: string; threadId: string | undefined };
  let trail: Place[] = $state([]);
  let at = $state(-1);
  /** Set while back or forward is being applied, so following the trail does
   *  not append to it. */
  let retracing = false;
  /** Long enough to cover a session's wandering, short enough to bound. */
  const TRAIL_LIMIT = 100;

  const canGoBack = $derived(at > 0);
  const canGoForward = $derived(at >= 0 && at < trail.length - 1);

  $effect(() => {
    const channelId = store.activeChannel();
    const threadId = store.activeThread();
    if (!channelId || retracing) return;
    const here = trail[at];
    // The same place twice is not a move: a rebuild, a refresh or reopening the
    // thread already open must not fill the trail with duplicates.
    if (here && here.channelId === channelId && here.threadId === threadId) return;
    // Anything ahead is abandoned the moment the reader goes somewhere new,
    // which is what every browser does and what makes forward mean anything.
    const kept = trail.slice(Math.max(0, at + 1 - TRAIL_LIMIT), at + 1);
    trail = [...kept, { channelId, threadId }];
    at = trail.length - 1;
  });

  /** Goes to a place already in the trail, without recording it as a new one. */
  async function retrace(index: number) {
    const place = trail[index];
    if (!place) return;
    retracing = true;
    at = index;
    try {
      if (store.activeChannel() !== place.channelId) {
        await select(place.channelId);
      }
      store.setActiveThread(place.threadId);
      // Held until the effect above has seen the new state; clearing sooner
      // lets it read a half-applied place and record it as a fresh one.
      await tick();
    } finally {
      retracing = false;
    }
    log.debug("nav.retraced", { index, trail: trail.length, threaded: Boolean(place.threadId) });
  }

  $effect(() => {
    // The two gestures everyone already has for this: the mouse's side buttons
    // and Alt with an arrow. Both are handled on the window rather than on the
    // stream, because "back" means the same thing wherever the focus is.
    const onKey = (event: KeyboardEvent) => {
      if (!event.altKey || event.ctrlKey || event.metaKey) return;
      if (event.key === "ArrowLeft" && canGoBack) {
        event.preventDefault();
        void retrace(at - 1);
      } else if (event.key === "ArrowRight" && canGoForward) {
        event.preventDefault();
        void retrace(at + 1);
      }
    };
    // `pointerdown`, not `click`: Windows sends the side buttons as button 3
    // and 4, and nothing else in the app claims them.
    const onPointer = (event: PointerEvent) => {
      if (event.button === 3 && canGoBack) {
        event.preventDefault();
        void retrace(at - 1);
      } else if (event.button === 4 && canGoForward) {
        event.preventDefault();
        void retrace(at + 1);
      }
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("pointerdown", onPointer);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("pointerdown", onPointer);
    };
  });

  /** The team whose public channels are being browsed, if any. */
  let browsingTeam: { id: string; name: string } | null = $state(null);
  /** Whether the "who do you want to talk to" picker is open. */
  let startingConversation = $state(false);

  /** The channel whose "leave" is being confirmed. */
  let leaving = $state("");

  async function leave(channelId: string) {
    const meId = store.meta()?.meId;
    if (!meId) return;
    leaving = "";
    optionsOpen = false;
    try {
      await api.leaveChannel(channelId, meId);
      log.info("channel.left", { channel: channelId });
      const meta = store.meta();
      if (meta) {
        const list = await api.refreshMembership(meta.meId, meta.displayMode);
        store.setChannelList(list);
        // Somewhere to be: the channel just left is gone from the sidebar, and
        // leaving the reader looking at it would show a channel they are no
        // longer in.
        const next = list.find((entry) => entry.id !== channelId);
        if (next) await select(next.id);
      }
    } catch (thrown) {
      error = `Could not leave that channel: ${String(thrown)}`;
      log.failure("channel.leave.failed", thrown, { channel: channelId });
    }
  }

  let searchOpen = $state(false);
  /** The saved or pinned panel, when one is open. Shares the search pane's
   *  column: two lists side by side would leave the stream too narrow to read,
   *  and nobody consults both at once. */
  let keptMode: "saved" | "pinned" | null = $state(null);
  /** Sets the reader's own presence. Do Not Disturb also silences toasts --
   *  the policy lives in Rust, and this is its input. */
  async function chooseStatus(status: string) {
    statusPickerOpen = false;
    const meId = store.meta()?.meId;
    if (!meId) return;
    try {
      ownStatus = await api.setMyStatus(meId, status);
      store.setStatus(meId, ownStatus);
      log.info("status.set", { status: ownStatus });
    } catch (thrown) {
      log.failure("status.set.failed", thrown, { status });
    }
  }

  /** The statuses a reader can set. The server's own values, so nothing has to
   *  be translated on the way out. */
  const STATUS_CHOICES: [value: string, label: string][] = [
    ["online", "Online"],
    ["away", "Away"],
    ["dnd", "Do not disturb"],
    ["offline", "Offline"],
  ];

  /** The profile card, opened from a mention, an avatar or an author's name. */
  let profileFor: { username: string; at: { x: number; y: number } } | null = $state(null);

  /** Follows a `~channel` written in a message.
   *
   *  A message can name a channel this reader is not in -- a private one, or a
   *  team they left -- and that is ordinary rather than an error, so it says so
   *  instead of failing silently. */
  async function followChannelLink(name: string) {
    try {
      const channelId = await api.channelByName(name);
      if (channelId) {
        await select(channelId);
      } else {
        error = `You are not in ~${name}.`;
      }
    } catch (thrown) {
      log.failure("channel.link.failed", thrown, {});
    }
  }

  /** This reader's own presence, and whether its picker is open. */
  let ownStatus = $state("online");
  let statusPickerOpen = $state(false);

  /** Who needs a presence dot: the sidebar's conversations and the authors on
   *  screen.
   *
   *  Derived as a *set* rather than fetched from an effect over the rows. The
   *  first version did the latter and asked for 45 statuses three times in the
   *  first second, and would have asked again on every arriving message --
   *  because the rows change constantly while the people in them almost never
   *  do. */
  const presenceWanted = $derived.by(() => {
    const wanted = new Set<string>();
    // The sidebar is where presence is actually read: whether somebody is
    // around decides whether you write to them now.
    for (const channel of channels) {
      if (channel.counterpart_id) wanted.add(channel.counterpart_id);
    }
    const channelId = store.activeChannel();
    for (const row of (channelId ? store.rowsOf(channelId) : undefined) ?? []) {
      if (row.kind === "post" || row.kind === "continuation") wanted.add(row.post.author_id);
    }
    return [...wanted].sort();
  });

  /** The set last asked for, so an unchanged set costs no request. */
  let presenceAsked = "";

  async function learnPresence(userIds: string[]) {
    if (userIds.length === 0) return;
    try {
      const found = await api.statuses(userIds);
      store.learnStatuses(found);
      log.debug("presence", { asked: userIds.length, got: Object.keys(found).length });
    } catch (error) {
      log.warn("presence.failed", { error: String(error) });
    }
  }

  /** What the header field last submitted, handed to the pane to run. */
  let searchSeed = $state("");
  let headerQuery = $state("");
  /** A message to reveal once its channel's history reaches back far enough. */
  let revealing: string | null = $state(null);

  /** Goes to a message found by search.
   *
   *  The plan is paged from the newest post backwards, so a hit from last month
   *  is simply not in the rows yet -- this pages back until it is, bounded,
   *  because "keep fetching until found" against a year of history is not a
   *  jump, it is a download. A proper jump fetches a window around the post and
   *  splices it in as an island; that is the permalink workstream, and until it
   *  lands this is honest about how far it will go.
   */
  /** The channel currently showing a jumped-to window instead of the present.
   *
   *  An island is disjoint from the newest page, and the gap between them is
   *  what must not be forgotten: rendering the two ranges together would show
   *  them as adjacent and put a silent hole in the scrollback. So while this is
   *  set, the live page is left alone entirely -- `refreshNewest` does not
   *  touch this channel -- and the way out is deliberate, through "Jump to
   *  newest". */
  let island: string | null = $state(null);

  /** How many posts to bring down on each side of a jumped-to message. */
  const ISLAND_SPAN = 60;

  async function revealPost(channelId: string, postId: string) {
    if (store.activeChannel() !== channelId) await select(channelId);
    revealing = postId;

    // Already on screen -- a search result in the page being read -- so nothing
    // needs fetching and the present stays where it is.
    if (virtual.indexOfPost(store.rowsOf(channelId) ?? [], postId) < 0) {
      try {
        const found = await api.fetchAround(channelId, postId, ISLAND_SPAN);
        // One page spanning the whole window. `since` is its floor and the
        // limit is generous, because the window is already bounded by what was
        // fetched.
        const page = await fetchWindow(channelId, found.oldest, found.newest);
        if (!page) throw new Error("the window came back empty");
        store.setPages(channelId, [pageOf(page)]);
        // A window that reached the newest post is not an island: it joins the
        // live page, so the channel goes back to following the present.
        island = found.reaches_present ? null : channelId;
        log.info("post.island", {
          channel: channelId,
          rows: page.rows.length,
          reachesPresent: found.reaches_present,
        });
      } catch (thrown) {
        revealing = null;
        log.failure("post.reveal.failed", thrown, { channel: channelId });
        error = `Could not reach that message: ${String(thrown)}`;
        return;
      }
    }

    await tick();
    const rows = store.rowsOf(channelId) ?? [];
    const index = virtual.indexOfPost(rows, postId);
    if (index < 0) {
      revealing = null;
      log.info("post.reveal.absent", { channel: channelId });
      error = "That message is not in this channel any more.";
      return;
    }
    const layout = virtual.layoutOf(channelId, rows);
    if (scroller) {
      // A third of the way down, so there is context above it.
      setScrollTop(
        Math.max(0, virtual.offsetOf(layout, index) - scroller.clientHeight / 3),
        "reveal",
      );
      readViewport();
    }
    log.info("post.revealed", { channel: channelId, index, island: island !== null });
    // Long enough to see where it landed, then the highlight goes.
    setTimeout(() => {
      if (revealing === postId) revealing = null;
    }, 2500);
  }

  /** One page covering an already-fetched window, bounded at both ends. */
  async function fetchWindow(channelId: string, oldest: number, newest: number) {
    const current = store.meta();
    if (!current) return undefined;
    return api.channelRows(
      channelId,
      current.meId,
      current.threadMode,
      current.displayMode,
      // `anchor` is the top bound and `since` the floor, so the two together
      // are exactly the window that was fetched. The +1 includes the newest
      // post itself, which `before` excludes.
      newest + 1,
      oldest,
      store.unreadSinceOf(channelId),
      store.NEWEST_PAGE_CEILING,
      store.fullRes(),
    );
  }

  /** Leaves a jumped-to window and returns to the live page. */
  async function leaveIsland() {
    const channelId = store.activeChannel();
    if (!channelId) return;
    island = null;
    store.clearPages(channelId);
    store.clearFloor(channelId);
    await refreshNewest(channelId);
    scrollToBottom();
    log.info("post.island.left", { channel: channelId });
  }

  /** The channel composer's file intake, bound out of it so a drop over the
   *  message list lands in the same tray. */
  let acceptDrop: ((files: File[]) => void) | undefined = $state();
  /** A file is being dragged over the conversation. */
  let dropping = $state(false);

  /** Groups with the team heading to draw above them, if any. */
  const sections = $derived.by(() => {
    const all = arranged;
    let previous = "";
    return all.map((group) => {
      // Drawn once per team, including when there is only one.
      //
      // It used to be suppressed for a single team as a heading nobody needed,
      // which was true until the team row became where a team's channels are
      // browsed from -- and a server with one team would then have had nowhere
      // to browse from at all.
      const heading = group.team_name && group.team_name !== previous ? group.team_name : "";
      if (group.team_name) previous = group.team_name;
      return { group, heading };
    });
  });

  const COLLAPSED_KEY = "matterless.collapsedGroups";

  function rememberedCollapse(): Set<string> {
    try {
      const raw = localStorage.getItem(COLLAPSED_KEY);
      return new Set<string>(raw ? (JSON.parse(raw) as string[]) : []);
    } catch {
      return new Set<string>();
    }
  }

  let collapsed = $state(rememberedCollapse());

  function toggleGroup(id: string) {
    const next = new Set(collapsed);
    if (!next.delete(id)) next.add(id);
    collapsed = next;
    try {
      localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...next]));
    } catch {
      // The collapse still applies to this session.
    }
  }

  /** What a collapsed group is hiding, so it can still say it. */
  function rollup(group: api.SidebarGroup) {
    return tally(group.channels);
  }

  /** What a set of channels is holding, ignoring the muted ones.
   *
   *  The same sum for a collapsed category and for a whole team, deliberately:
   *  a reader comparing the two would otherwise be comparing two different
   *  definitions of "unread". */
  function tally(list: api.ChannelSummary[]) {
    let messages = 0;
    let mentions = 0;
    for (const channel of list) {
      const unread = store.unreadOf(channel.id);
      if (!unread || unread.muted) continue;
      messages += unread.messages;
      mentions += unread.mentions;
    }
    return { messages, mentions };
  }

  /** The teams contributing to the sidebar, in the order they appear there.
   *
   *  Derived from the groups rather than fetched: the sidebar already knows
   *  which teams it is drawing, and a second source would be a second answer. */
  const teams = $derived.by(() => {
    const seen = new Map<string, string>();
    for (const group of arranged) {
      if (group.team_id && !seen.has(group.team_id)) {
        seen.set(group.team_id, group.team_name || "");
      }
    }
    return [...seen].map(([id, name]) => ({ id, name }));
  });

  /** The team the open channel belongs to, so the rail can say where you are. */
  const activeTeam = $derived(channels.find((channel) => channel.id === active)?.team_id ?? "");

  let sidebar: HTMLElement | undefined = $state();

  /** Scrolls the sidebar to a team's heading.
   *
   *  A rail that *scrolls* rather than one that filters: this sidebar shows
   *  every team at once, deliberately, so a rail that hid the others would be a
   *  different sidebar rather than an addition to this one. This is a way to
   *  reach a team quickly without losing sight of the rest.
   */
  function goToTeam(teamId: string) {
    const heading = sidebar?.querySelector<HTMLElement>(`[data-team="${teamId}"]`);
    if (!heading) return;
    heading.scrollIntoView({ block: "start", behavior: "smooth" });
    log.debug("sidebar.team.reached", { team: teamId });
  }

  /** What each team is holding, by team id.
   *
   *  Derived over every group rather than the visible ones, because a team's
   *  count has to be true whatever is collapsed -- the whole point of putting a
   *  number on the team row is that it survives its categories being shut. */
  const teamUnread = $derived.by(() => {
    const totals = new Map<string, { messages: number; mentions: number }>();
    for (const group of arranged) {
      if (!group.team_id) continue;
      const held = totals.get(group.team_id) ?? { messages: 0, mentions: 0 };
      const sum = tally(group.channels);
      held.messages += sum.messages;
      held.mentions += sum.mentions;
      totals.set(group.team_id, held);
    }
    return totals;
  });

  // A rearrangement in another client arrives as a burst: one event per
  // category touched. One regroup covers all of them.
  const SIDEBAR_COALESCE_MS = 250;
  let sidebarTimer: number | undefined;

  function scheduleSidebar(teamId: string) {
    if (sidebarTimer !== undefined) return;
    sidebarTimer = window.setTimeout(() => {
      sidebarTimer = undefined;
      log.debug("sidebar.regrouping", { team: teamId });
      void loadSidebar();
    }, SIDEBAR_COALESCE_MS);
  }

  /// Re-reads membership after a menu action changed it, and moves off a
  /// channel the reader is no longer in.
  async function refreshChannels() {
    const meta = store.meta();
    if (!meta) return;
    try {
      const list = await api.refreshMembership(meta.meId, meta.displayMode);
      store.setChannelList(list);
      // Leaving from the menu can take the channel being read out from under
      // it, which the sidebar would show but the stream would not.
      if (active && !list.some((entry) => entry.id === active)) {
        const next = list[0];
        if (next) await select(next.id);
      }
    } catch (error) {
      log.warn("membership.refresh.failed", { error: String(error) });
    }
  }

  async function loadSidebar() {
    const meta = store.meta();
    if (!meta) return;
    try {
      const fetched = await api.sidebar(meta.meId, meta.displayMode);
      store.setSidebarGroups(fetched.groups);
      store.setSeparateUnreads(fetched.separate_unreads);
      // The server's own collapse state seeds the first run only: after that
      // the reader's own toggles are what matter.
      log.info("sidebar.grouped", {
        groups: fetched.groups.length,
        channels: fetched.groups.reduce((total, group) => total + group.channels.length, 0),
        separateUnreads: fetched.separate_unreads,
      });
    } catch (error) {
      log.warn("sidebar.failed", { error: String(error) });
    }
  }
  const info = $derived(store.meta());
  const live = $derived(store.connected());

  /** A websocket frame waiting to become pixels, keyed by channel.
   *
   *  Phase 3's gate is the time from a frame arriving to the reader seeing it,
   *  so the two halves have to be stitched: Rust times the frame up to the
   *  delta, the shell times the delta up to the frame callback after the rows
   *  mount. Only one trace per channel is kept -- a burst of deltas coalesces
   *  into a single rebuild, and the oldest frame is the honest start for it.
   */
  interface FrameTrace {
    wsMs: number;
    ipcMs: number;
    receivedAt: number;
  }
  const inFlight = new Map<string, FrameTrace>();

  /** Fetches one page of rows: the newest page, or the page below `before`.
   *
   *  The newest page is bounded below by its floor rather than by a count, so
   *  arriving messages join it instead of pushing its oldest posts out of it.
   *  Its limit is a safety net, not the definition.
   */
  /** What the last page fetch cost, split by where the time went.
   *
   *  `build` is Rust's own figure for producing the plan; `round` is everything
   *  between asking and having it in JavaScript -- serialising the payload,
   *  crossing the IPC boundary, and parsing it back. Nothing measured that gap
   *  before, and a page of 454 rows carries parsed nodes for every post. */
  let lastFetch = { ms: 0, build: 0, round: 0, rows: 0, bytes: 0 };

  async function fetchPage(
    channelId: string,
    before: number | null,
  ): Promise<api.RowsPayload | undefined> {
    const current = store.meta();
    if (!current) return undefined;
    const newest = before === null;
    const floor = newest ? store.floorOf(channelId) : undefined;
    const asked = performance.now();
    const payload = await api.channelRows(
      channelId,
      current.meId,
      current.threadMode,
      current.displayMode,
      before,
      floor ?? null,
      store.unreadSinceOf(channelId),
      // The ceiling belongs to a *pinned* page, whose real bound is its floor.
      // The first fetch of a visit has no floor yet, so it is the count that
      // decides -- asking for the ceiling there read 2000 posts and built one
      // 2685-row page, which is the opposite of paging.
      floor === undefined ? store.PAGE_POSTS : store.NEWEST_PAGE_CEILING,
      store.fullRes(),
    );
    const ms = performance.now() - asked;
    // Stringified only to size the payload, and only on a fetch we are about to
    // report: it is not cheap on a page this size, which is rather the point.
    const bytes = JSON.stringify(payload).length;
    lastFetch = {
      ms,
      build: payload.build_ms,
      round: ms - payload.build_ms,
      rows: payload.rows.length,
      bytes,
    };
    return payload;
  }

  /** Rebuilds the newest page, which is the only one a live message can change.
   *
   *  Older pages are left exactly as they were: their posts cannot change
   *  without an edit, which arrives as its own delta for its own page. That is
   *  the whole point of paging -- a message arriving used to cost a rebuild of
   *  every row the reader had scrolled back through.
   */
  async function refreshNewest(channelId: string) {
    // A channel showing a jumped-to window is not following the present, and
    // the newest page must not be joined onto it: the two ranges have a gap
    // between them, and rendering them together would show it as no gap at all.
    // The reader leaves the island deliberately, and that is what resumes this.
    if (island === channelId) {
      log.debug("page.island.held", { channel: channelId });
      return;
    }
    const page = await fetchPage(channelId, null);
    if (!page) return;
    // The first page of a visit fixes the floor; from then on the same posts
    // stay in the same page.
    if (store.floorOf(channelId) === undefined && page.oldest_in_window !== null) {
      store.setFloor(channelId, page.oldest_in_window);
    }

    const pages = store.pagesOf(channelId);
    const last = pages[pages.length - 1];
    const fetched = pageOf(page);

    // Nothing above the floor is not the same as nothing in the channel.
    //
    // Sealing a full page moves the floor above its newest post, on purpose, so
    // the next message starts a fresh small page. Until that message arrives
    // this request legitimately matches no posts -- and the two callers that
    // rebuild the open channel a second after bootstrap (the emoji catalogue
    // and the thread totals) both hit exactly that window. Painting the empty
    // result replaced a 454-row page with nothing and the reader was told the
    // channel was empty.
    if (page.rows.length === 0 && pages.length > 0) {
      log.debug("page.nothing_newer", { channel: channelId, pages: pages.length });
      return;
    }

    // Replace the last page only when it *is* this page. After a seal the
    // fetched window starts above the sealed one, and replacing it would throw
    // the sealed page's rows away -- the same wipe, just deferred to whenever
    // the next message arrived.
    const sameWindow = last !== undefined && last.oldest === fetched.oldest;
    const next =
      pages.length === 0
        ? [fetched]
        : sameWindow
          ? [...pages.slice(0, -1), fetched]
          : [...pages, fetched];
    paint(channelId, next, page, "newest");

    // Watching a busy channel long enough would grow the newest page without
    // limit, and with it the cost of every rebuild. Past two pages' worth, seal
    // it: the floor moves above its newest post, so the next message starts a
    // fresh small page and this one is never rebuilt again.
    if (page.rows.length > 2 * store.PAGE_POSTS && page.newest_in_window !== null) {
      store.setFloor(channelId, page.newest_in_window + 1);
      log.info("page.sealed", { channel: channelId, rows: page.rows.length });
    }
  }

  function pageOf(payload: api.RowsPayload): store.Page {
    return {
      rows: payload.rows,
      oldest: payload.oldest_in_window,
    };
  }

  /** Applies a new set of pages and owns the scroll position while doing it.
   *
   *  The single place scroll is touched on a rebuild. Moving it from two places
   *  is what made the earlier sliding window cascade.
   */
  function paint(
    channelId: string,
    pages: store.Page[],
    payload: api.RowsPayload,
    reason: "newest" | "older",
  ) {
    const wasAtBottom = atBottom();
    const heightBefore = scroller?.scrollHeight ?? 0;
    const topBefore = scroller?.scrollTop ?? 0;
    const startedAt = performance.now();
    let anchoredBy = "position";

    // Which row the reader is looking at, by identity, before the rows change.
    //
    // A prepend used to be anchored by *arithmetic* -- shift the scroll by
    // however much the content grew -- and that is only correct if the added
    // rows are the height the layout guessed. They are not: 190 rows arrive
    // unmeasured, and when their real heights land the accumulated error moves
    // the reader. Measured: `anchored=position` followed by a 195px correction,
    // which is the jump this is here to remove. Anchoring on the row itself is
    // immune, because the offset is recomputed from whatever the layout now
    // says.
    const held = (() => {
      if (!scroller || reason !== "older") return undefined;
      const before = store.rowsOf(channelId) ?? [];
      if (before.length === 0) return undefined;
      const at = scroller.scrollTop;
      const layout = virtual.layoutOf(channelId, before);
      const index = virtual.indexAt(layout, before.length, at);
      const row = before[index];
      if (!row) return undefined;
      return { key: virtual.rowKey(row), within: at - virtual.offsetOf(layout, index) };
    })();

    // Reference data, accumulated: the names this page resolved stay known when
    // the reader moves on.
    store.learnEmoji(payload.emoji);
    store.setPages(channelId, pages);
    const rows = store.rowsOf(channelId) ?? [];

    requestAnimationFrame(() => {
      if (scroller) {
        if (settling?.channelId === channelId) {
          // The opening scroll owns the position until it lands on the divider.
          // Without this, a refresh landing mid-open read "was at the bottom"
          // from the previous channel and jumped to the newest message.
          readViewport();
          requestAnimationFrame(settleOpening);
        } else if (reason === "newest" && wasAtBottom) {
          setScrollTop(scroller.scrollHeight, "page.newest");
        } else if (held) {
          // Put the same row back where it was, wherever the new layout says it
          // now lives.
          const after = store.rowsOf(channelId) ?? [];
          const layout = virtual.layoutOf(channelId, after);
          const index = after.findIndex((row) => virtual.rowKey(row) === held.key);
          // Restoring by key only works if keys are unique, and a bug that
          // breaks that uniqueness stays invisible until the reader is thrown
          // thousands of pixels off their place. Cheap enough to keep as a
          // guard: the cause was two pages each emitting a separator for the
          // same calendar day.
          if (import.meta.env.DEV) {
            const seen = new Set<string>();
            const dupes = new Set<string>();
            for (const row of after) {
              const key = virtual.rowKey(row);
              if (seen.has(key)) dupes.add(key);
              seen.add(key);
            }
            if (dupes.size > 0) {
              log.debug("rows.duplicate_keys", {
                rows: after.length,
                dupes: dupes.size,
                worst: [...dupes].slice(0, 4).join(" "),
              });
            }
          }
          if (index >= 0) {
            setScrollTop(virtual.offsetOf(layout, index) + held.within, "page.identity");
            anchoredBy = "identity";
          } else {
            // The row is gone -- a tombstone, or a page that resealed. Fall back
            // to the growth shift rather than leaving the view where it was.
            setScrollTop(topBefore + (scroller.scrollHeight - heightBefore), "page.lost");
          }
        } else {
          // Rows were added above the viewport: shift by exactly the growth so
          // the content under the cursor does not move.
          setScrollTop(topBefore + (scroller.scrollHeight - heightBefore), "page.growth");
        }
        readViewport();
      }

      const trace = inFlight.get(channelId);
      if (trace) {
        inFlight.delete(channelId);
        const uiMs = performance.now() - trace.receivedAt;
        // The number Phase 3 actually gates on. Logged rather than shown: it is
        // for reading a run afterwards, not for watching while you work.
        log.info("latency.ws_to_glyph", {
          channel: channelId,
          totalMs: (trace.wsMs + trace.ipcMs + uiMs).toFixed(1),
          wsMs: trace.wsMs.toFixed(1),
          ipcMs: trace.ipcMs.toFixed(1),
          uiMs: uiMs.toFixed(1),
          buildMs: payload.build_ms.toFixed(1),
          rows: rows.length,
        });
      }
      log.debug("rows.rendered", {
        channel: channelId,
        reason,
        pages: pages.length,
        rows: rows.length,
        pageRows: payload.rows.length,
        buildMs: payload.build_ms.toFixed(2),
        paintMs: (performance.now() - startedAt).toFixed(2),
        pageMs: payload.timings.page_ms.toFixed(2),
        threadsMs: payload.timings.threads_ms.toFixed(2),
        parseMs: payload.timings.parse_ms.toFixed(2),
        cacheMs: payload.timings.cache_ms.toFixed(2),
        planMs: payload.timings.plan_ms.toFixed(2),
        hits: payload.timings.cache_hits,
        misses: payload.timings.cache_misses,
        pageFull: payload.window_full,
        anchored: reason === "newest" && wasAtBottom ? "bottom" : anchoredBy,
      });
    });
  }

  // ---- the thread pane's width, dragged and remembered ------------------
  //
  // A proportion of the window was the wrong default: it made the pane grow on
  // a big screen, where the extra room is better spent on the channel. A fixed
  // starting width that the reader can drag is both smaller and theirs.
  const THREAD_DEFAULT = 300;
  const THREAD_MIN = 240;
  const THREAD_MAX = 680;
  /** Never more than this share of the window, so a remembered width from a
   *  maximised session cannot swallow a small one. */
  const THREAD_SHARE = 0.55;
  const THREAD_KEY = "matterless.threadWidth";

  function clampThread(px: number): number {
    const ceiling = Math.min(THREAD_MAX, Math.round(window.innerWidth * THREAD_SHARE));
    return Math.max(THREAD_MIN, Math.min(px, Math.max(THREAD_MIN, ceiling)));
  }

  function rememberedThreadWidth(): number {
    try {
      const raw = localStorage.getItem(THREAD_KEY);
      const parsed = raw === null ? NaN : Number.parseInt(raw, 10);
      return clampThread(Number.isFinite(parsed) ? parsed : THREAD_DEFAULT);
    } catch {
      // A blocked or empty store is not a failure: the default is fine.
      return THREAD_DEFAULT;
    }
  }

  let threadWidth = $state(rememberedThreadWidth());
  let dragging = $state(false);

  function storeThreadWidth(px: number) {
    try {
      localStorage.setItem(THREAD_KEY, String(px));
    } catch {
      // Not worth telling anyone: the width still applies to this session.
    }
  }

  function startThreadDrag(event: PointerEvent) {
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    dragging = true;
    event.preventDefault();
  }

  function dragThread(event: PointerEvent) {
    if (!dragging) return;
    // Measured from the right edge, which is the edge the pane is pinned to.
    threadWidth = clampThread(window.innerWidth - event.clientX);
  }

  function endThreadDrag(event: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
    storeThreadWidth(threadWidth);
    log.debug("thread.width", { px: threadWidth });
  }

  /** Keyboard nudges, so the split is not mouse-only. */
  function nudgeThread(event: KeyboardEvent) {
    const step = event.shiftKey ? 40 : 12;
    if (event.key === "ArrowLeft") threadWidth = clampThread(threadWidth + step);
    else if (event.key === "ArrowRight") threadWidth = clampThread(threadWidth - step);
    else return;
    event.preventDefault();
    storeThreadWidth(threadWidth);
  }

  let scroller: HTMLElement | undefined = $state();
  // What the list needs to decide which rows to mount. Read from the scroller
  // on every scroll event: cheap, and the alternative -- the list observing the
  // scroller itself -- would put scroll knowledge in two places.
  let scrollTop = $state(0);
  /** The scroller's full height, for our own scrollbar to size its thumb. */
  let contentHeight = $state(0);

  $effect(() => {
    // The stream changes height without anyone scrolling: a message arrives, a
    // page of history lands, rows re-measure. Read on scroll alone, the height
    // the scrollbar sizes itself against goes stale between gestures.
    store.activeChannel();
    const content = scroller?.firstElementChild;
    if (!content) return;
    const observer = new ResizeObserver(() => readViewport());
    observer.observe(content);
    return () => observer.disconnect();
  });
  let viewportHeight = $state(0);

  function readViewport() {
    if (!scroller) return;
    scrollTop = scroller.scrollTop;
    viewportHeight = scroller.clientHeight;
    contentHeight = scroller.scrollHeight;
  }

  /** The last scroll position this shell wrote, so its own writes can be told
   *  apart from the reader's. */
  let ourScrollWrite = -1;
  /** When a wheel or a key last moved the view. */
  let lastGesture = 0;
  /** While set, the reader is dragging the scrollbar and owns the position. */
  let draggingUntil = 0;
  /** The same thing as state, because the list has to know as well. */
  let thumbDragging = $state(false);
  let dragSettle: number | undefined;

  /** Whether the newest message is in view, for the jump button. */
  let nearBottom = $state(true);
  /** Whether the oldest loaded message is in view, for the history notice. */
  let nearTop = $state(false);

  /** A wheel or a key means the reader is following the *content*, so keeping
   *  their row still is right. Recorded, because a drag is then anything else
   *  that moves the scroll and was not us. */
  function noteGesture() {
    lastGesture = performance.now();
  }

  /** Scroll work is coalesced to one frame.
   *
   *  A scrollbar drag is driven on the *main thread* in Chromium, so anything
   *  slow in a scroll handler shows up as the thumb lagging behind the cursor --
   *  and this handler recomputes the visible slice, which mounts and unmounts
   *  rows. Scroll events fire faster than frames, so doing that per event is
   *  work the reader pays for in latency and cannot see the benefit of. */
  let scrollPending = false;

  function onScroll() {
    if (scrollPending) return;
    scrollPending = true;
    requestAnimationFrame(() => {
      scrollPending = false;
      if (!scroller) return;
      nearBottom = atBottom();
      nearTop = scroller.scrollTop < 120;
      readViewport();
      void maybeLoadOlder();
    });
  }

  /** Every write to the scroller's position goes through here.
   *
   *  One funnel so `ourScrollWrite` can never be forgotten: it was set by hand
   *  at nine call sites, and a write that misses it reads to the scroll handler
   *  as the reader having scrolled. */
  function setScrollTop(value: number, why: string) {
    if (!scroller) return;
    scroller.scrollTop = value;
    ourScrollWrite = scroller.scrollTop;
    lastScrollReason = why;
  }

  /** What last moved the view, for the log to name when something looks wrong. */
  let lastScrollReason = "";

  function atBottom() {
    if (!scroller) return true;
    return scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
  }

  function scrollToBottom() {
    queueMicrotask(() => {
      if (!scroller) return;
      setScrollTop(scroller.scrollHeight, "to.bottom");
      readViewport();
    });
  }

  /** How far below the top the "New messages" line sits when a channel opens:
   *  enough context above it to see what the messages are replying to. */
  const DIVIDER_MARGIN = 60;
  /** Frames the opening scroll keeps correcting itself before settling for
   *  close enough. Half a second at 60Hz. */
  const SETTLE_FRAMES = 30;

  /** The channel currently opening on its "New messages" line, if any.
   *
   *  Armed for the whole opening sequence rather than applied once. One write
   *  could never be right: the first attempt is against *estimated* heights for
   *  hundreds of rows nobody has measured, and everything that lands afterwards
   *  -- a measured row, the refreshed page arriving -- moves the line again.
   *
   *  It also has to own the scroll while it is armed. `paint` decides whether to
   *  stick to the newest message by reading `atBottom()` off the scroller, and
   *  on a channel switch that reads the *previous* channel: opening on the
   *  divider was landing correctly and then being yanked to the bottom a frame
   *  later by the refresh painting the newest page.
   */
  let settling:
    | { channelId: string; frames: number; steady: number; height: number }
    | undefined;

  /** Frames the position and the content height must both hold before the
   *  opening scroll is considered landed.
   *
   *  One frame is not enough: the newest page is already anchored at the bottom
   *  when the settle starts, so the very first check finds nothing to correct
   *  and lands -- while every row is still an estimate. The heights then move,
   *  and with `settling` already cleared the anchor restore takes over and
   *  jumps to the end, which is a visible scroll down on every switch. */
  const STEADY_FRAMES = 3;

  /** Laid out but not shown, while the opening scroll finds its position.
   *
   *  Opening a channel paints two or three different layouts before it comes to
   *  rest: the position is corrected after the first frame is already on screen
   *  (measured: +453px in one case, +217px in another), and the content height
   *  bulges for a single frame as rows mount (14874 -> 15550 -> 14878), which
   *  moves everything on screen without the scroll position moving at all.
   *
   *  `visibility` rather than `display` or a mount gate: the rows have to be
   *  laid out and measured for the settle to converge at all -- they simply do
   *  not need to be watched doing it.
   *
   *  Kept deliberately brief. It covers the first paint or two, where the big
   *  correction lands; holding it for the whole settle turned a small movement
   *  into a blank window, which is worse. The rest of the settle is corrected
   *  in `requestAnimationFrame`, which runs before the frame is painted, so
   *  those corrections are not seen either way. */
  let opening = $state(false);
  let openingRelease: number | undefined;

  /** How long the stream may stay hidden. Settling normally takes two or three
   *  frames; this is the cap for a channel whose heights never quite agree, so
   *  a slow one still appears promptly rather than staying blank. */
  const OPENING_CAP_MS = 45;

  /** Opens at the first new message when there is one, otherwise at the newest.
   *
   *  `cover` hides the stream while the position settles. Right for the switch
   *  itself, where the content is being replaced anyway; wrong for the reconcile
   *  pass that follows a few hundred milliseconds later, because hiding a stream
   *  the reader is already reading is a blink rather than a cover. */
  function scrollToFirstUnread(cover: boolean) {
    const channelId = store.activeChannel();
    if (!channelId) return;
    settling = {
      channelId,
      frames: 0,
      steady: 0,
      height: scroller?.scrollHeight ?? 0,
    };
    if (cover) {
      opening = true;
      clearTimeout(openingRelease);
      openingRelease = window.setTimeout(() => (opening = false), OPENING_CAP_MS);
    }
    requestAnimationFrame(settleOpening);
  }

  /** The reader took over, or the position landed: stop correcting, wherever we
   *  had got to, and show the result. */
  function abandonOpening() {
    settling = undefined;
    opening = false;
    clearTimeout(openingRelease);
  }

  /** One correction towards the divider, repeated until it is exact.
   *
   *  Exact means the divider row is *mounted*: only then does its position come
   *  from the DOM rather than from estimates of rows the virtualiser has never
   *  shown, and only then is there nothing left to correct.
   */
  function settleOpening() {
    const target = settling;
    if (!target || !scroller) return;
    const channelId = store.activeChannel();
    if (channelId !== target.channelId) {
      abandonOpening();
      return;
    }

    const rows = store.rowsOf(channelId) ?? [];
    const index = virtual.dividerIndex(rows);
    if (index < 0) {
      // Nothing unread: the newest message is where to be.
      //
      // Repeated until it holds, the same way the divider case is. One write to
      // the bottom does not *stay* at the bottom: the rows below are estimates
      // at that moment, and as they mount and measure the total height moves,
      // so the position that was the end no longer is. Landing early here also
      // cleared `settling`, which is what hands the view to the anchor restore
      // -- and that pins whatever row is under the viewport, parking the reader
      // short of the newest message with "Jump to newest" showing.
      const furthest = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
      const adriftFromEnd = Math.abs(scroller.scrollTop - furthest);
      if (adriftFromEnd >= 1) {
        setScrollTop(furthest, "settle.newest");
        readViewport();
      }
      target.frames += 1;
      // Steady means nothing left to correct *and* a content height that has
      // stopped moving. Either alone is satisfied on the first frame, while the
      // rows below are still estimates.
      const stillGrowing = scroller.scrollHeight !== target.height;
      target.height = scroller.scrollHeight;
      target.steady = adriftFromEnd < 1 && !stillGrowing ? target.steady + 1 : 0;
      if (target.steady >= STEADY_FRAMES || target.frames > SETTLE_FRAMES) {
        // Logged because "it did not go to the new messages" and "there were
        // none" look identical on screen, and this is the difference.
        log.debug("opened.at.newest", {
          channel: channelId,
          rows: rows.length,
          frames: target.frames,
        });
        abandonOpening();
        return;
      }
      requestAnimationFrame(settleOpening);
      return;
    }

    const mounted = scroller.querySelector<HTMLElement>("[data-unread-divider]");
    const offset = mounted
      ? mounted.getBoundingClientRect().top -
        scroller.getBoundingClientRect().top +
        scroller.scrollTop
      : virtual.offsetOf(virtual.layoutOf(channelId, rows), index);
    const furthest = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
    const desired = Math.min(Math.max(0, offset - DIVIDER_MARGIN), furthest);
    const adrift = Math.abs(scroller.scrollTop - desired);

    // Landed: the line is where it should be, and the DOM is what said so.
    if (mounted && adrift < 1) {
      log.debug("opened.at.divider", {
        index,
        offset: offset.toFixed(0),
        frames: target.frames,
      });
      abandonOpening();
      return;
    }
    if (adrift >= 1) {
      setScrollTop(desired, "settle.divider");
      readViewport();
    }

    target.frames += 1;
    if (target.frames > SETTLE_FRAMES) {
      // Close enough beats fighting the layout forever -- and says so, because
      // "it nearly worked" is otherwise invisible.
      log.debug("opened.near.divider", { index, adrift: adrift.toFixed(0) });
      abandonOpening();
      return;
    }
    requestAnimationFrame(settleOpening);
  }

  /** Re-reads the scroll position after the list corrects an estimate.
   *
   *  Measuring a row that was taller than its estimate moves every offset below
   *  it, which moves what is under the reader's cursor unless the slice is
   *  recomputed from the new layout.
   */
  function onLayoutChanged(anchor: { key: string; within: number }) {
    if (!scroller) return;
    // While a channel is opening, the divider is the anchor -- and a measured
    // row is exactly the event that moved it.
    if (settling && settling.channelId === store.activeChannel()) {
      requestAnimationFrame(settleOpening);
      return;
    }
    // Heights settle over the first frames after a channel opens. If the reader
    // is at the newest message, keep them there while that happens -- otherwise
    // every correction below the viewport nudges the newest post off screen.
    // The reader is dragging: they are choosing where to be, and holding a row
    // still would undo it.
    if (performance.now() < draggingUntil) {
      readViewport();
      return;
    }
    const stick = atBottom();
    const channelId = store.activeChannel();

    // Applied immediately: the list awaited `tick()` before reporting, so the
    // corrected spacers are already in the DOM and the write is clamped against
    // the *new* scroll height. Deferring this by a frame -- which an earlier
    // version did, for exactly that clamping reason -- meant the browser
    // painted the displaced frame first, which is what read as the list
    // rewinding while paging back through history.
    {
      if (!scroller) return;
      if (stick) {
        setScrollTop(scroller.scrollHeight, "layout.stick");
        readViewport();
        return;
      }
      if (!channelId) return;
      const rows = store.rowsOf(channelId) ?? [];
      const index = rows.findIndex((row) => virtual.rowKey(row) === anchor.key);
      if (index < 0) {
        readViewport();
        return;
      }
      const layout = virtual.layoutOf(channelId, rows);
      const target = virtual.offsetOf(layout, index) + anchor.within;
      const correction = target - scroller.scrollTop;
      // A sub-pixel difference is not worth a write: it would fight the
      // browser's own scrolling for no visible gain.
      if (Math.abs(correction) >= 1) {
        setScrollTop(target, "layout.anchor");
      }
      // Only the ones big enough to see, so the log says whether estimates are
      // improving without drowning in one-pixel noise.
      if (Math.abs(correction) >= 8) {
        log.debug("scroll.corrected", {
          by: correction.toFixed(0),
          row: index,
          // What moved the view last: a correction chasing our own write is a
          // loop, and this is what names the writer.
          after: lastScrollReason,
        });
      }
      readViewport();
    }
  }

  let loadingOlder = $state(false);
  let exhausted = $state(new Set<string>());

  /** Extends history backwards when the reader approaches the top.
   *
   *  Scroll position is *not* touched here. `loadRows` preserves the reader's
   *  visual position by the height delta, which is the only place that logic
   *  should live -- moving the scroll from two places is what made the sliding
   *  window cascade.
   */
  async function maybeLoadOlder() {
    const channelId = store.activeChannel();
    if (!channelId || loadingOlder) return;
    if (!scroller || scroller.scrollTop > 300) return;
    // Never mid-drag. A page is ~190 rows of new height arriving above the
    // viewport, which moves the thumb under the cursor by more than anything
    // else here -- and the reader is mid-gesture, so it also decides where they
    // land. `Scrollbar`'s `ondragging` asks again the moment the thumb is
    // released; without that call this early return means a drag to the top
    // never pages.
    if (thumbDragging) return;

    const pages = store.pagesOf(channelId);
    const oldestPage = pages[0];
    if (!oldestPage || oldestPage.oldest === null) return;
    if (exhausted.has(channelId)) return;

    loadingOlder = true;
    try {
      // No gate: history goes back as far as the server holds it. The only stop
      // is running out of posts, which `history.exhausted` records.
      //
      // A server fetch is 200 *posts*, but under collapsed threads only roots
      // become rows -- and in a channel of long threads that can be a handful.
      // Measured: 200 posts yielding 3 rows, so one fetch per scroll meant
      // paging back three messages at a time. Keep fetching until the page is
      // worth showing, bounded so one gesture cannot run away.
      const WANT_ROWS = 25;
      const MAX_FETCHES = 12;
      let page = await fetchPage(channelId, oldestPage.oldest);
      let fetches = 0;
      let ranOut = false;
      while ((page?.rows.length ?? 0) < WANT_ROWS && fetches < MAX_FETCHES) {
        const more = await api.loadOlder(channelId);
        fetches += 1;
        if (!more) {
          ranOut = true;
          break;
        }
        page = await fetchPage(channelId, oldestPage.oldest);
      }
      if (ranOut && (page?.rows.length ?? 0) === 0) {
        exhausted = new Set([...exhausted, channelId]);
        log.info("history.exhausted", { channel: channelId, pages: pages.length });
        return;
      }
      if (!page || page.rows.length === 0) return;
      log.debug("history.paged", {
        channel: channelId,
        rows: page.rows.length,
        fetches,
        reachedStart: ranOut,
      });
      if (ranOut) exhausted = new Set([...exhausted, channelId]);
      paint(channelId, [pageOf(page), ...pages], page, "older");
    } catch (thrown) {
      log.failure("history.older.failed", thrown, { channel: channelId });
    } finally {
      loadingOlder = false;
    }
  }

  /** How long a channel may take to open before the reader is told it did not.
   *
   *  Generous -- opening measures 46-111ms end to end -- because this exists
   *  only to bound a request that never settles at all. A rejected invoke logs
   *  and shows an error; a *pending* one used to leave the stream on its
   *  placeholder for ever with nothing in the log to say why. */
  const OPEN_TIMEOUT_MS = 8000;

  /** Rejects if `work` has not settled in time. The pending promise is left to
   *  finish on its own: there is nothing to cancel, and its result is only
   *  ignored. */
  function withTimeout<T>(work: Promise<T>, within: number, what: string): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const timer = window.setTimeout(
        () => reject(new Error(`${what} did not answer within ${within}ms`)),
        within,
      );
      work.then(
        (value) => {
          clearTimeout(timer);
          resolve(value);
        },
        (thrown) => {
          clearTimeout(timer);
          reject(thrown);
        },
      );
    });
  }

  /** Opens a channel: cached rows first, then a refresh if one is warranted. */
  const LAST_CHANNEL_KEY = "matterless.lastChannel";
  const FULL_RES_KEY = "matterless.fullResImages";

  /** Restored before the first row request, so the plan is built for the
   *  rendition it is going to draw. */
  function rememberedFullRes(): boolean {
    try {
      return localStorage.getItem(FULL_RES_KEY) === "true";
    } catch {
      return false;
    }
  }

  async function setFullRes(on: boolean) {
    store.setFullRes(on);
    try {
      localStorage.setItem(FULL_RES_KEY, String(on));
    } catch {
      // The choice still holds for this session.
    }
    log.info("settings.fullRes", { on });
    // Which rendition is drawn is baked into the plan, so the pages have to be
    // built again -- and only the pages that exist, not the whole history: a
    // fresh visit rebuilds the rest on its own.
    const channelId = store.activeChannel();
    if (channelId) {
      store.setPages(channelId, []);
      store.clearFloor(channelId);
      await refreshNewest(channelId);
    }
  }

  function rememberedChannel(): string | undefined {
    try {
      return localStorage.getItem(LAST_CHANNEL_KEY) ?? undefined;
    } catch {
      return undefined;
    }
  }

  async function select(channelId: string) {
    store.setActiveChannel(channelId);
    try {
      localStorage.setItem(LAST_CHANNEL_KEY, channelId);
    } catch {
      // The choice still holds for this session.
    }
    // A thread belongs to the channel it was opened from.
    store.setActiveThread(undefined);
    // A visit starts in the present, whatever the last one was looking at.
    if (island !== null && island !== channelId) island = null;
    // A fresh visit starts from one page again; scrolling back builds the rest.
    //
    // Cleared *before* the open request, not after: leaving the last visit's
    // rows on screen to avoid the blank was tried and showed content that did
    // not match the channel. Cleared to *unknown* rather than to empty, so the
    // list says it is loading instead of claiming the channel has no messages.
    store.clearPages(channelId);
    store.clearFloor(channelId);
    const startedAt = performance.now();

    let opened: api.ChannelOpened;
    try {
      opened = await withTimeout(api.openChannel(channelId), OPEN_TIMEOUT_MS, "open_channel");
    } catch (thrown) {
      // Without this the rejection was swallowed by the `void select(...)` at
      // every call site: the list kept its placeholder for ever and the log
      // said nothing at all.
      log.failure("channel.open.failed", thrown, { channel: channelId });
      error = `Could not open that channel: ${String(thrown)}`;
      return;
    }
    // Held for the visit, so marking read does not erase the divider while it
    // is still being read.
    store.setUnreadSince(channelId, opened.divider_at);
    log.info("channel.selected", { channel: channelId });
    const openedAt = performance.now();
    await refreshNewest(channelId);
    log.debug("channel.open.phases", {
      totalMs: (performance.now() - startedAt).toFixed(0),
      openMs: (openedAt - startedAt).toFixed(0),
      rowsMs: (performance.now() - openedAt).toFixed(0),
      fetchMs: lastFetch.ms.toFixed(1),
      buildMs: lastFetch.build.toFixed(1),
      roundMs: lastFetch.round.toFixed(1),
      rows: lastFetch.rows,
      kb: (lastFetch.bytes / 1024).toFixed(0),
    });
    scrollToFirstUnread(true);
    if (opened.needs_refresh) {
      // Painted from SQLite already; the network only corrects it.
      await api.refresh(channelId);
      // Not covered: the stream is already on screen by now.
      scrollToFirstUnread(false);
    }
  }

  // A burst of unread deltas (a resync sweeps every channel) is one badge
  // redraw, not one per channel.
  const BADGE_COALESCE_MS = 150;
  let badgeTimer: number | undefined;

  function scheduleBadge() {
    if (badgeTimer !== undefined) return;
    badgeTimer = window.setTimeout(async () => {
      badgeTimer = undefined;
      const meId = store.meta()?.meId;
      if (!meId) return;
      try {
        const badge = await api.updateBadge(meId);
        log.debug("badge", {
          anyUnread: badge.any_unread,
          attention: badge.attention,
          mentions: badge.mentions,
          threadMentions: badge.thread_mentions,
          followed: badge.followed_unread,
        });
      } catch (error) {
        log.warn("badge.failed", { error: String(error) });
      }
    }, BADGE_COALESCE_MS);
  }

  function onDelta(delta: api.UiDelta) {
    log.debug("delta", { kind: delta.kind });
    switch (delta.kind) {
      case "channel": {
        // The delta names the channel; the newest page is refetched. One path in.
        const rendered = store.pagesOf(delta.channel_id).length > 0;
        if (rendered && delta.ws_ms !== null && !inFlight.has(delta.channel_id)) {
          inFlight.set(delta.channel_id, {
            wsMs: delta.ws_ms,
            // Wall clock either side of the IPC hop, since the shell cannot see
            // when it started. Coarse, and the only term here that is.
            ipcMs: Math.max(0, Date.now() - (delta.emitted_at_ms ?? Date.now())),
            receivedAt: performance.now(),
          });
        }
        if (rendered) void refreshNewest(delta.channel_id);
        break;
      }
      case "unread": {
        const before = store.unreadOf(delta.channel_id);
        const grew = delta.messages > (before?.messages ?? 0);
        store.setUnread(delta.channel_id, {
          messages: delta.messages,
          mentions: delta.mentions,
          muted: delta.muted,
        });
        // Asking for attention you already have is how an app becomes
        // irritating, so the flash is for an unfocused window only. A muted
        // channel never flashes -- muting says "do not interrupt me".
        if (grew && !delta.muted && !focused) {
          const urgent = delta.mentions > (before?.mentions ?? 0);
          void api.flashTaskbar(urgent);
        }
        scheduleBadge();
        break;
      }
      case "typing":
        store.noteTyping(delta.channel_id, delta.user_id, delta.root_id || undefined);
        break;
      case "sidebar":
        // Dragging one channel emits several of these, so the regroup is
        // coalesced rather than run per event.
        scheduleSidebar(delta.team_id);
        break;
      case "status":
        store.setStatus(delta.user_id, delta.status);
        if (delta.user_id === store.meta()?.meId) ownStatus = delta.status;
        break;
      case "connection":
        store.setConnected(delta.connected);
        break;
      case "notify":
        // Raised unconditionally: the policy already ran in Rust, so second-
        // guessing it here would mean two places deciding the same thing.
        void toast.raise(
          delta.channel_id,
          delta.author,
          delta.channel,
          delta.preview,
          delta.direct,
        );
        break;
    }
  }

  // What a mention or a `~channel` in a message body does. Registered here
  // because `Nodes` is recursive and threading two callbacks through every
  // branch of it would be noise at each level.
  store.onMessageLinks({
    profile: (username, at) => (profileFor = { username, at }),
    channel: (name) => void followChannelLink(name),
    // A quoted message opens where it was said, through the same reveal the
    // search pane uses -- including its honest limit on how far back it pages.
    permalink: (channelId, postId) => void revealPost(channelId, postId),
  });

  async function start() {
    // Subscribe first: bootstrap is what starts the websocket, so subscribing
    // afterwards can miss the connection event entirely.
    // Before the first row request: the setting decides which rendition the
    // plan reserves a box for.
    store.setFullRes(rememberedFullRes());
    await api.subscribe(onDelta);
    const boot = await api.bootstrap();
    store.setMeta({
      meId: boot.me_id,
      username: boot.username,
      threadMode: boot.thread_mode,
      displayMode: boot.display_mode,
      maxFileSize: boot.max_file_size,
    });
    // The level, not an edge: if the socket came up while bootstrap was still
    // running, its delta has already been and gone.
    store.setConnected(boot.connected);
    ownStatus = boot.own_status;
    store.setChannelList(boot.channels);
    for (const channel of boot.channels) {
      store.setUnread(channel.id, {
        messages: channel.unread,
        mentions: channel.mentions,
        muted: channel.muted,
      });
    }
    // The badge is drawn from what bootstrap just loaded, so a window that
    // starts minimised already shows the right thing.
    scheduleBadge();

    // Then reconcile: a cached sidebar carries last session's counts, so the
    // badge and the channel list stay wrong until the server is asked. Not
    // awaited -- the window is already usable.
    void api
      .refreshMembership(boot.me_id, boot.display_mode)
      .then((channels) => {
        for (const channel of channels) {
          store.setUnread(channel.id, {
            messages: channel.unread,
            mentions: channel.mentions,
            muted: channel.muted,
          });
        }
        store.setChannelList(channels);
        log.info("membership.refreshed", { channels: channels.length });
        scheduleBadge();
      })
      .catch((error) => log.warn("membership.refresh.failed", { error: String(error) }));

    // Followed threads, for the per-thread unread the footers show. Not awaited
    // either: the channel is already on screen, and thread counts are an
    // annotation on it rather than a precondition for it.
    // The grouped sidebar, which needs one request per team.
    void loadSidebar();

    // The whole custom emoji set, once: it is what stops a `:name:` appearing
    // for an emoji simply because nobody had used it here yet.
    void api
      .refreshEmoji()
      .then((catalogue) => {
        // Learned wholesale, so a custom emoji renders wherever it appears --
        // a channel, a thread reply, a reaction pill -- rather than only where
        // some page happened to scan its name.
        store.learnEmoji(catalogue);
        log.info("emoji.catalogued", { custom: Object.keys(catalogue).length });
        const channelId = store.activeChannel();
        if (channelId) void refreshNewest(channelId);
      })
      .catch((error) => log.warn("emoji.catalogue.failed", { error: String(error) }));

    void api
      .refreshThreads(boot.me_id)
      .then((totals) => {
        log.info("threads.refreshed", {
          followed: totals.followed,
          unreadThreads: totals.unread_threads,
          unreadMentions: totals.unread_mentions,
        });
        // Footer counts live in the row plan, so the open channel is rebuilt.
        const channelId = store.activeChannel();
        if (channelId) void refreshNewest(channelId);
        scheduleBadge();
      })
      .catch((error) => log.warn("threads.refresh.failed", { error: String(error) }));

    log.info("bootstrapped", {
      channels: boot.channels.length,
      fromCache: boot.from_cache,
      // Logged because "offline over a live socket" was invisible from here:
      // the connection delta is an edge and can land before the shell has
      // subscribed, so this level is what the badge is actually drawn from.
      connected: boot.connected,
      threadMode: boot.thread_mode,
      displayMode: boot.display_mode,
    });

    // Where the reader left off, then anything unread, then the first channel.
    // Opening on whatever happened to be unread meant a restart moved you
    // somewhere you had not chosen.
    const remembered = rememberedChannel();
    const opening =
      boot.channels.find((channel) => channel.id === remembered) ??
      boot.channels.find((channel) => channel.unread > 0) ??
      boot.channels[0];
    // Startup and a click behave identically: the dwell timer decides the rest.
    if (opening) await select(opening.id);
  }

  /** The server this install talks to, empty until one is chosen. */
  let server = $state("");
  /** What is typed into the server field before it is accepted. */
  let serverDraft = $state("");

  async function chooseServer(event: Event) {
    event.preventDefault();
    busy = true;
    error = "";
    try {
      server = await api.setServer(serverDraft);
      log.info("server.chosen", {});
    } catch (thrown) {
      error = String(thrown);
      log.failure("server.choose.failed", thrown);
    } finally {
      busy = false;
    }
  }

  async function submit(event: Event) {
    event.preventDefault();
    busy = true;
    error = "";
    try {
      // The credentials go up again with the code, because a Mattermost login
      // is one call either way -- there is no half-authenticated state on the
      // server to attach the second factor to.
      const outcome = await api.signIn(loginId, password, mfaNeeded ? mfaToken : undefined);
      if (outcome.outcome === "mfa_required") {
        mfaNeeded = true;
        log.info("sign.in.mfa.asked", {});
        return;
      }
      log.info("signed.in", { username: outcome.username });
      password = "";
      mfaToken = "";
      mfaNeeded = false;
      signedIn = true;
      await start();
    } catch (thrown) {
      error = String(thrown);
      log.failure("sign.in.failed", thrown, { mfa: mfaNeeded });
    } finally {
      busy = false;
    }
  }

  /** Back to the credentials, discarding a code that was never accepted. */
  function backToCredentials() {
    mfaNeeded = false;
    mfaToken = "";
    error = "";
  }

  /** Puts the caret in the code box the moment that screen appears. */
  function takeFocus(node: HTMLInputElement) {
    node.focus();
  }

  // A channel is marked read only after this much *focused* time in front of
  // it. Counting focused time only is the whole point: a window sitting in the
  // background must never silently clear an unread badge.
  const DWELL_BEFORE_READ_MS = 2500;
  const TICK_MS = 250;

  let focused = $state(true);
  let dwellMs = $state(0);

  /** Are the unread messages actually on screen?
   *
   *  Being in the channel is not the same as having seen them: scrolled up in
   *  history, the new messages are below the fold and marking them read would
   *  be a lie. The "New messages" line is the boundary -- if it is above the
   *  bottom of the viewport, what follows it is in view. Without a divider row
   *  (under collapsed threads the unread may all be replies) the newest post
   *  being visible is the best available answer.
   */
  function newMessagesVisible(channelId: string): boolean {
    if (!scroller) return false;
    const rows = store.rowsOf(channelId) ?? [];
    const index = virtual.dividerIndex(rows);
    if (index < 0) return atBottom();
    const layout = virtual.layoutOf(channelId, rows);
    return (
      virtual.offsetOf(layout, index) < scroller.scrollTop + scroller.clientHeight
    );
  }

  $effect(() => {
    const timer = setInterval(() => {
      const channelId = store.activeChannel();
      if (!channelId || !focused || !newMessagesVisible(channelId)) {
        // Scrolling away pauses *and* resets: the two and a half seconds are
        // meant to be spent looking at the messages, not accumulated across
        // glances at something else.
        dwellMs = 0;
        return;
      }
      dwellMs += TICK_MS;
      if (dwellMs < DWELL_BEFORE_READ_MS) return;
      // Keeps firing while unread remains, so messages arriving as you watch
      // are cleared too rather than piling up on the channel you are reading.
      const unread = store.unreadOf(channelId)?.messages ?? 0;
      if (unread > 0) {
        log.debug("channel.marking.read", { channel: channelId, dwellMs, unread });
        void api.markRead(channelId).then(scheduleBadge);
      }
    }, TICK_MS);
    return () => clearInterval(timer);
  });

  $effect(() => {
    // One request when the *set* changes, coalesced: opening a channel adds its
    // authors a page at a time, and three requests for the same 45 people is
    // three too many. The websocket keeps them current afterwards, which is why
    // there is no polling timer here.
    const signature = presenceWanted.join(",");
    if (signature === presenceAsked || presenceWanted.length === 0) return;
    const asking = presenceWanted;
    const timer = window.setTimeout(() => {
      presenceAsked = signature;
      void learnPresence(asking);
    }, 250);
    return () => clearTimeout(timer);
  });



  $effect(() => {
    // The browser's own account of the main thread being blocked. A scrollbar
    // drag is driven on that thread in Chromium, so a long task *is* the thumb
    // freezing -- this says whether the lag is ours and how long for.
    if (typeof PerformanceObserver === "undefined") return;
    let observer: PerformanceObserver | undefined;
    try {
      observer = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          if (entry.duration >= 60) {
            log.debug("main.thread.blocked", { ms: entry.duration.toFixed(0) });
          }
        }
      });
      observer.observe({ entryTypes: ["longtask"] });
    } catch {
      // Not every build reports long tasks; nothing here depends on it.
    }
    return () => observer?.disconnect();
  });

  $effect(() => {
    // Ctrl+K is the switcher, as everywhere else. Handled on the window rather
    // than on an element so it works while the composer has focus, which is
    // where the caret usually is.
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        switcherOpen = !switcherOpen;
      } else if (
        (event.ctrlKey || event.metaKey) &&
        event.shiftKey &&
        event.key.toLowerCase() === "f"
      ) {
        event.preventDefault();
        keptMode = null;
        searchOpen = !searchOpen;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  $effect(() => {
    // Clicking a toast focuses the window in Rust and asks here for the
    // channel: navigation belongs to the shell, and the window may well have
    // been minimised with a different channel open.
    const stop = listen<string>("toast-clicked", (event) => {
      const channelId = event.payload;
      log.info("toast.clicked", { channel: channelId });
      if (store.activeChannel() !== channelId) void select(channelId);
    });
    return () => void stop.then((off) => off());
  });

  $effect(() => {
    // A slice cannot be computed without a viewport height, and it changes when
    // the window is resized.
    readViewport();
    const onResize = () => {
      readViewport();
      // A width remembered from a wider window has to shrink back into this one.
      threadWidth = clampThread(threadWidth);
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  });

  $effect(() => {
    const gainedFocus = () => {
      focused = true;
      void api.setFocus(true);
      // The window has been looked at, so it stops asking to be.
      void api.clearAttention();
    };
    const lostFocus = () => {
      focused = false;
      void api.setFocus(false);
    };
    window.addEventListener("focus", gainedFocus);
    window.addEventListener("blur", lostFocus);
    focused = document.hasFocus();
    return () => {
      window.removeEventListener("focus", gainedFocus);
      window.removeEventListener("blur", lostFocus);
    };
  });

  void (async () => {
    try {
      const current = await api.session();
      // Empty until an install has been pointed at one, which is what puts the
      // sign-in screen into asking for a server before anything else.
      server = current.server;
      signedIn = current.signed_in;
      if (current.signed_in) await start();
    } catch (thrown) {
      error = String(thrown);
      signedIn = false;
      log.failure("startup.failed", thrown);
    }
  })();
</script>

<!-- The browser's own menu -- Back, Refresh, Save as, Print -- belongs to a
     browser, and none of it does anything useful in a chat client. It is kept
     only where it is the way to cut, copy and paste: inside an editable field,
     or over a live selection. Anywhere else the webview underneath should not
     be visible. -->
<svelte:window
  oncontextmenu={(event) => {
    const target = event.target as HTMLElement | null;
    if (target?.closest("input, textarea, [contenteditable='true']")) return;
    if ((window.getSelection()?.toString() ?? "").trim().length > 0) return;
    event.preventDefault();
  }}
/>

{#if signedIn === undefined}
  <main class="centre"><p>Starting…</p></main>
{:else if !signedIn}
  <main class="centre">
    {#if !server}
      <!-- Which server comes first, because everything else depends on it:
           there is no host compiled into this app, and the token that follows
           belongs to whichever one is chosen here. -->
      <form onsubmit={chooseServer}>
        <h1>MatterLess</h1>
        <p class="hint">Which Mattermost server?</p>
        <label>
          Server
          <input
            bind:value={serverDraft}
            placeholder="mattermost.example.com"
            autocomplete="url"
            spellcheck="false"
          />
        </label>
        <button type="submit" disabled={busy || !serverDraft.trim()}>
          {busy ? "Checking…" : "Continue"}
        </button>
        {#if error}<p class="error">{error}</p>{/if}
      </form>
    {:else}
      <!-- The code is a screen of its own, and only for the accounts that are
           asked for one. A field labelled "if asked" makes every reader decide
           whether it applies to them; the server already knows. -->
      <form onsubmit={submit}>
        <h1>MatterLess</h1>
        {#if mfaNeeded}
          <p class="hint">
            {loginId} has two-factor sign-in. Enter the code from your authenticator.
          </p>
          <label
            >Authentication code<input
              bind:value={mfaToken}
              use:takeFocus
              inputmode="numeric"
              autocomplete="one-time-code"
              spellcheck="false"
            /></label
          >
          <button type="submit" disabled={busy || !mfaToken.trim()}>
            {busy ? "Checking…" : "Sign in"}
          </button>
          <p class="hint">
            <button type="button" class="linkish" onclick={backToCredentials}>Start over</button>
          </p>
        {:else}
          <p class="hint">
            Signing in to {server}.
            <button type="button" class="linkish" onclick={() => (server = "")}>change</button>
          </p>
          <label>Email or username<input bind:value={loginId} autocomplete="username" /></label>
          <label
            >Password<input
              type="password"
              bind:value={password}
              autocomplete="current-password"
            /></label
          >
          <button type="submit" disabled={busy}>{busy ? "Signing in…" : "Sign in"}</button>
        {/if}
        {#if error}<p class="error">{error}</p>{/if}
      </form>
    {/if}
  </main>
{:else}
  <div
    class="shell"
    class:threaded={openThread}
    class:searching={searchOpen || keptMode !== null}
    style:--thread-width="{threadWidth}px"
  >
    <aside>
      <div class="who">
        <strong>{info?.username ?? ""}</strong>
        <!-- The reader's own presence, and the only control for it. Its
             values are the server's: online, away, do not disturb, offline. -->
        <button
          type="button"
          class="own-status {ownStatus}"
          aria-expanded={statusPickerOpen}
          title="Your status"
          onclick={() => (statusPickerOpen = !statusPickerOpen)}
        >
          <span class="own-dot"></span>
          {ownStatus === "dnd" ? "do not disturb" : ownStatus}
        </button>
        {#if statusPickerOpen}
          <div class="status-picker" role="dialog" aria-label="Set your status">
            {#each STATUS_CHOICES as [value, label] (value)}
              <button
                type="button"
                class:current={ownStatus === value}
                onclick={() => void chooseStatus(value)}
              >
                <span class="own-dot {value}"></span>{label}
              </button>
            {/each}
          </div>
        {/if}
        <span class="badge" class:live>
          {live ? "live" : "offline"}
        </span>
      </div>
      <div class="sidebar-body">
        {#if teams.length > 1}
          <!-- Only worth a rail when there is more than one team to reach. -->
          <div class="rail" role="tablist" aria-label="Teams">
            {#each teams as team (team.id)}
              {@const held = teamUnread.get(team.id)}
              <button
                type="button"
                role="tab"
                aria-selected={activeTeam === team.id}
                class:current={activeTeam === team.id}
                title={team.name}
                aria-label={team.name}
                onclick={() => goToTeam(team.id)}
              >
                <img
                  src={media.team(team.id)}
                  alt=""
                  onerror={(event) => (event.currentTarget as HTMLImageElement).remove()}
                />
                <span class="initial">{(team.name || "?").slice(0, 1).toUpperCase()}</span>
                {#if (held?.mentions ?? 0) > 0}
                  <span class="pip mention">{held?.mentions}</span>
                {:else if (held?.messages ?? 0) > 0}
                  <span class="pip"></span>
                {/if}
              </button>
            {/each}
          </div>
        {/if}
      <nav bind:this={sidebar}>
        {#snippet channelRow(channel: api.ChannelSummary)}
          {@const unread = store.unreadOf(channel.id)}
          {@const asking = (unread?.messages ?? 0) > 0 && !unread?.muted}
          <button
            type="button"
            class:selected={active === channel.id}
            class:muted={unread?.muted}
            class:unread={asking}
            onclick={() => select(channel.id)}
            oncontextmenu={(event) => {
              event.preventDefault();
              channelMenu = { channel, x: event.clientX, y: event.clientY };
            }}
          >
            <ChannelIcon {channel} />
            <span class="name">{channel.display_name}</span>
            {#if (unread?.mentions ?? 0) > 0}
              <span class="count mention">{unread?.mentions}</span>
            {:else if (unread?.messages ?? 0) > 0}
              <span class="count">{unread?.messages}</span>
            {/if}
          </button>
        {/snippet}

        {#if sections.length}
          {#each sections as { group, heading } (group.id)}
            {#if heading}
              <p class="team" data-team={group.team_id}>
                <!-- A team icon is optional on the server, so this falls back
                     to the team's initials rather than a broken image. -->
                <img
                  class="team-icon"
                  src={media.team(group.team_id)}
                  alt=""
                  onerror={(event) => (event.currentTarget as HTMLImageElement).remove()}
                />
                <span>{heading}</span>
                {#if (teamUnread.get(group.team_id)?.mentions ?? 0) > 0}
                  <!-- A team's own count, so a team whose categories are all
                       collapsed still says it is holding something. Mentions
                       win over messages: they are the ones being asked for. -->
                  <span class="count mention"
                    >{teamUnread.get(group.team_id)?.mentions}</span
                  >
                {:else if (teamUnread.get(group.team_id)?.messages ?? 0) > 0}
                  <span class="count">{teamUnread.get(group.team_id)?.messages}</span>
                {/if}
                <!-- Browsing belongs to the *team*, not to a category: this
                     team's channels are spread across favourites and custom
                     categories as well as the one called "Channels", so hanging
                     it off that category would have offered only part of the
                     team it claimed to browse. -->
                <button
                  type="button"
                  class="add"
                  title="Browse channels in {heading}"
                  aria-label="Browse channels in {heading}"
                  onclick={() => (browsingTeam = { id: group.team_id, name: heading })}>＋</button
                >
              </p>
            {/if}
            {@const hidden = collapsed.has(group.id)}
            {@const hiding = rollup(group)}
            <!-- The row is a heading with an action beside it, not one
                 control: collapsing the category and adding to it are different
                 things, and a button cannot be nested inside a button. -->
            <div class="group-row" class:unreads={group.category_type === "unreads"}>
            <button
              class="group"
              class:unreads={group.category_type === "unreads"}
              type="button"
              onclick={() => toggleGroup(group.id)}
            >
              <span class="twist" class:open={!hidden}>›</span>
              <span class="label">{group.display_name}</span>
              {#if hidden && hiding.mentions > 0}
                <span class="count mention">{hiding.mentions}</span>
              {:else if hidden && hiding.messages > 0}
                <!-- A collapsed group still says what it is hiding. -->
                <span class="count">{hiding.messages}</span>
              {/if}
            </button>
            <!-- Where each belongs: browsing is about *this team's* channels,
                 and the group already carries its team, so the button is per
                 team without anything having to work out which one. Starting a
                 conversation belongs beside the conversations. -->
            {#if group.category_type === "direct_messages"}
              <button
                type="button"
                class="add"
                title="New conversation"
                aria-label="New conversation"
                onclick={() => (startingConversation = true)}>＋</button
              >
            {/if}
            </div>
            {#if !hidden}
              {#each group.channels as channel (channel.id)}
                {@render channelRow(channel)}
              {/each}
            {/if}
          {/each}
        {:else}
          <!-- Before the grouping request lands: the flat list from bootstrap,
               so the sidebar is never empty. -->
          {#each channels as channel (channel.id)}
            {@render channelRow(channel)}
          {/each}
        {/if}
      </nav>
      </div>
    </aside>
    <main>
      {#if active}
        {@const channel = channels.find((entry) => entry.id === active)}
        {#if channel}
          <!-- Which conversation this is, always on screen: with 114 channels
               and a thread pane over the top, the sidebar selection is not
               always in view. -->
          <header class="channel">
            <!-- Where you were, and where you were before that. Disabled rather
                 than hidden: a control that comes and goes is one you cannot
                 learn the position of. -->
            <span class="trail">
              <button
                type="button"
                class="step"
                title="Back (Alt+Left)"
                aria-label="Back"
                disabled={!canGoBack}
                onclick={() => void retrace(at - 1)}>‹</button
              >
              <button
                type="button"
                class="step"
                title="Forward (Alt+Right)"
                aria-label="Forward"
                disabled={!canGoForward}
                onclick={() => void retrace(at + 1)}>›</button
              >
            </span>
            <ChannelIcon {channel} size={18} />
            <strong>{channel.display_name}</strong>
            {#if store.unreadOf(active)?.muted}
              <span class="muted-note">muted</span>
            {/if}
            <!-- The way in, always on screen: Ctrl+Shift+F is for people who
                 already know the feature exists. Enter hands the text to the
                 pane and clears this box, so there is never a second live copy
                 of the query to disagree with the one being refined. -->
            <input
              class="search-field"
              type="search"
              bind:value={headerQuery}
              placeholder="Search messages…"
              spellcheck="false"
              onkeydown={(event) => {
                if (event.key === "Enter" && !event.isComposing) {
                  event.preventDefault();
                  const typed = headerQuery.trim();
                  if (!typed) return;
                  searchSeed = typed;
                  keptMode = null;
                  searchOpen = true;
                  headerQuery = "";
                } else if (event.key === "Escape") {
                  headerQuery = "";
                }
              }}
            />
            <button
              type="button"
              class="options-button"
              title="Pinned messages"
              aria-pressed={keptMode === "pinned"}
              onclick={() => {
                searchOpen = false;
                keptMode = keptMode === "pinned" ? null : "pinned";
              }}>📌</button
            >
            <button
              type="button"
              class="options-button"
              title="Saved messages"
              aria-pressed={keptMode === "saved"}
              onclick={() => {
                searchOpen = false;
                keptMode = keptMode === "saved" ? null : "saved";
              }}>🔖</button
            >
            <button
              type="button"
              class="options-button"
              aria-expanded={optionsOpen}
              title="Display options"
              onclick={() => (optionsOpen = !optionsOpen)}>⚙</button
            >
            {#if optionsOpen}
              <div class="options" role="dialog" aria-label="Display options">
                <label>
                  <input
                    type="checkbox"
                    checked={store.fullRes()}
                    onchange={(event) => void setFullRes(event.currentTarget.checked)}
                  />
                  Full-resolution images
                </label>
                <p>
                  A post with one picture is drawn from the file as uploaded rather than
                  the server's 1920-wide preview: sharper on a large screen, and several
                  megabytes each. Posts with several pictures always use thumbnails, drawn
                  at their own size.
                </p>
                <label>
                  <input
                    type="checkbox"
                    checked={store.previewImages()}
                    onchange={(event) => store.setPreviewImages(event.currentTarget.checked)}
                  />
                  Link preview images
                </label>
                <p>
                  This server runs no image proxy, so a preview image is fetched straight
                  from the site it belongs to — which tells that site someone here read the
                  message. The card still says what the page is without it.
                </p>
                {#if channel.channel_type === "O" || channel.channel_type === "P"}
                  <!-- Only for a real channel: the server refuses to remove
                       anyone from a direct or group message, which are left by
                       hiding them rather than by membership. -->
                  <hr />
                  {#if leaving === channel.id}
                    <!-- Asked, not assumed: leaving a private channel can mean
                         not being able to get back in. -->
                    <p class="confirm-leave">
                      Leave {channel.display_name}?
                      <button type="button" class="danger" onclick={() => void leave(channel.id)}
                        >Leave</button
                      >
                      <button type="button" onclick={() => (leaving = "")}>Stay</button>
                    </p>
                  {:else}
                    <button
                      type="button"
                      class="leave"
                      onclick={() => (leaving = channel.id)}>Leave channel…</button
                    >
                  {/if}
                {/if}
              </div>
            {/if}
          </header>
        {/if}
        <!-- A drop anywhere over the conversation goes to the composer's tray,
             which is where the official client puts it too. `dragDropEnabled`
             is false in tauri.conf.json, so the webview sees the drop rather
             than the window swallowing it. -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="scroll-frame">
        <div
          class="scroll"
          class:opening
          id="stream-scroll"
          class:dropping
          bind:this={scroller}
          onscroll={onScroll}
          onwheel={() => {
            abandonOpening();
            noteGesture();
          }}
          onpointerdown={abandonOpening}
          onkeydown={() => {
            abandonOpening();
            noteGesture();
          }}
          ondragover={(event) => {
            if (!event.dataTransfer?.types.includes("Files")) return;
            event.preventDefault();
            dropping = true;
          }}
          ondragleave={() => (dropping = false)}
          ondrop={(event) => {
            dropping = false;
            const files = Array.from(event.dataTransfer?.files ?? []);
            if (files.length === 0) return;
            event.preventDefault();
            acceptDrop?.(files);
          }}
        >
          <MessageList
            {rows}
            channelId={active ?? ""}
            {scrollTop}
            {viewportHeight}
            liveScrollTop={() => scroller?.scrollTop ?? 0}
            onlayout={onLayoutChanged}
            frozen={thumbDragging}
          />
        </div>
        {#if nearTop && active && (loadingOlder || exhausted.has(active))}
          <!-- At the top, "still fetching" and "that is all there is" look
               identical, and the difference decides whether to keep waiting.
               An overlay rather than a row: anything in the flow would add
               height the virtualiser has not reserved. -->
          <p class="history" aria-live="polite">
            {#if loadingOlder}
              Loading earlier messages…
            {:else}
              Beginning of the channel
            {/if}
          </p>
        {/if}
        <Scrollbar
          top={scrollTop}
          viewport={viewportHeight}
          content={contentHeight}
          onmove={(to) => {
            setScrollTop(to, "thumb");
            readViewport();
          }}
          ondragging={(active) => {
            thumbDragging = active;
            // Paging is suppressed while the thumb is held, so releasing it is
            // what has to ask again -- otherwise dragging to the top never
            // loads older history at all.
            if (!active) void maybeLoadOlder();
            // The thumb is now a real signal, so the guard that keeps height
            // corrections from undoing a drag can use it directly instead of
            // the 300ms heuristic that used to guess at one.
            draggingUntil = active ? Number.POSITIVE_INFINITY : performance.now() + 120;
          }}
        />
        </div>
        {#if island === active}
          <!-- Always offered while the view is a jumped-to window, not only
               when scrolled up: the bottom of an island is not the present, and
               nothing else on screen says so. -->
          <button type="button" class="to-newest" onclick={() => void leaveIsland()}>
            Viewing older messages · Jump to newest ↓
          </button>
        {:else if !nearBottom}
          <!-- Getting back to the newest message should not be a scrolling
               exercise: with a virtualised list of thousands of rows, dragging
               the thumb there takes several goes. -->
          <button type="button" class="to-newest" onclick={() => scrollToBottom()}>
            Jump to newest ↓
          </button>
        {/if}
        <!-- Always present, empty or not: appearing and disappearing moved the
             whole stream and the composer with it. The strip is the reserved
             space; only its text comes and goes. -->
        <p class="typing" aria-live="polite">
          {#if store.typingIn(active).length}
            {store.typingIn(active).length} typing…
          {/if}
        </p>
        <Composer
          channelId={active}
          maxFileSize={store.meta()?.maxFileSize ?? 0}
          bind:accept={acceptDrop}
        />
      {:else}
        <p class="hint">Pick a channel.</p>
      {/if}
      {#if error}<p class="error">{error}</p>{/if}
    </main>
    {#if channelMenu}
      <ChannelMenu
        channel={channelMenu.channel}
        at={{ x: channelMenu.x, y: channelMenu.y }}
        meId={info?.meId ?? ""}
        groups={(store.sidebarGroups() ?? []).filter(
          (group) => group.team_id === channelMenu!.channel.team_id,
        )}
        favorite={(store.sidebarGroups() ?? []).some(
          (group) =>
            group.category_type === "favorites" &&
            group.channels.some((held) => held.id === channelMenu!.channel.id),
        )}
        onaddmembers={() => (addingTo = channelMenu!.channel)}
        onchanged={() => {
          void loadSidebar();
          void refreshChannels();
        }}
        onclose={() => (channelMenu = null)}
      />
    {/if}
    {#if addingTo}
      <AddMembers
        channelId={addingTo.id}
        channelName={addingTo.display_name}
        onadded={() => void loadSidebar()}
        onclose={() => (addingTo = null)}
      />
    {/if}
    {#if startingConversation}
      <NewConversation
        onclose={() => (startingConversation = false)}
        onopen={(channelId) => {
          startingConversation = false;
          void select(channelId);
        }}
      />
    {/if}
    {#if browsingTeam}
      <BrowseChannels
        teamId={browsingTeam.id}
        teamName={browsingTeam.name}
        onclose={() => (browsingTeam = null)}
        onopen={(channelId) => {
          browsingTeam = null;
          void select(channelId);
        }}
      />
    {/if}
    {#if profileFor}
      <ProfileCard
        username={profileFor.username}
        at={profileFor.at}
        onclose={() => (profileFor = null)}
        onmessage={(channelId) => {
          profileFor = null;
          void select(channelId);
        }}
      />
    {/if}
    {#if switcherOpen}
      <Switcher
        onclose={() => (switcherOpen = false)}
        onpick={(channelId) => {
          switcherOpen = false;
          void select(channelId);
        }}
      />
    {/if}
    <!-- Both panes at once is one too many for any window this runs in, and
         search is the one that was just asked for. -->
    {#if openThread && !searchOpen}
      <!-- A focusable separator is ARIA's window splitter, which is why it
           carries its current value: arrow keys move it, so it has to say what
           it is set to. -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div
        class="divider"
        class:dragging
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize the thread pane"
        aria-valuenow={threadWidth}
        aria-valuemin={THREAD_MIN}
        aria-valuemax={THREAD_MAX}
        tabindex="0"
        onpointerdown={startThreadDrag}
        onpointermove={dragThread}
        onpointerup={endThreadDrag}
        onpointercancel={endThreadDrag}
        onkeydown={nudgeThread}
      ></div>
      <ThreadPane rootId={openThread} onclose={() => store.setActiveThread(undefined)} />
    {/if}
    {#if searchOpen}
      <Search
        initial={searchSeed}
        onclose={() => (searchOpen = false)}
        onjump={(channelId, postId) => void revealPost(channelId, postId)}
      />
    {:else if keptMode}
      <Kept
        mode={keptMode}
        channelId={active ?? ""}
        channelLabel={channels.find((entry) => entry.id === active)?.display_name ?? "this channel"}
        onclose={() => (keptMode = null)}
        onjump={(channelId, postId) => void revealPost(channelId, postId)}
      />
    {/if}
  </div>
{/if}

<style>
  :global(:root) {
    --ground: #eef2f7;
    --surface: #ffffff;
    --surface-2: #e3eaf1;
    --ink: #0f1720;
    --ink-soft: #526270;
    --ink-faint: #7a8896;
    --rule: #ccd8e2;
    /* A fainter rule, for separators inside a panel rather than between panes. */
    --rule-soft: #e0e8ef;
    --signal: #0a6178;
    --signal-soft: #d7ecf2;
    --flag: #8d4f07;
    /* Presence needs a "good" and a "stop": `--ok` was used by the dots before
       it existed anywhere, so `var(--ok)` resolved to nothing and an online
       marker rendered transparent -- indistinguishable from offline. */
    --ok: #166b53;
    --danger: #a5322a;
    --mono: "Cascadia Mono", ui-monospace, Consolas, monospace;
    color-scheme: light dark;
  }
  @media (prefers-color-scheme: dark) {
    :global(:root) {
      --ground: #0c1218;
      --surface: #141d26;
      --surface-2: #1b2734;
      --ink: #e3eaf1;
      --ink-soft: #94a4b3;
      --ink-faint: #6d7d8c;
      --rule: #253342;
      --rule-soft: #1c2733;
      --signal: #4bcbec;
      --signal-soft: #10323d;
      --flag: #e0a13f;
      --ok: #4ec49b;
      --danger: #e2685a;
    }
  }
  :global(body) {
    margin: 0;
    background: var(--ground);
    color: var(--ink);
    font-family: "Segoe UI", system-ui, sans-serif;
    font-size: 14px;
  }
  .shell {
    display: grid;
    /* The third column exists only while a thread is open, so the stream keeps
       the whole width the rest of the time. */
    grid-template-columns: 260px 1fr;
    height: 100vh;
  }
  .shell.threaded {
    grid-template-columns: 260px minmax(0, 1fr) 6px var(--thread-width);
  }
  /* Search takes a column of its own, and a narrower one than the thread pane:
     it holds three-line excerpts rather than a conversation. */
  .shell.searching {
    grid-template-columns: 260px minmax(0, 1fr) 320px;
  }
  /* Both at once is one pane too many for any window this app runs in, so
     search wins -- it is the one that was just asked for. */
  .shell.threaded.searching {
    grid-template-columns: 260px minmax(0, 1fr) 320px;
  }
  .shell.threaded.searching > .divider {
    display: none;
  }
  .divider {
    cursor: col-resize;
    background: var(--rule);
    /* Thin to look at, but the hit area is what makes it draggable rather than
       fiddly, so it borrows a few pixels either side. */
    box-shadow: 0 0 0 3px transparent;
    touch-action: none;
  }
  .divider:hover,
  .divider:focus-visible,
  .divider.dragging {
    background: var(--signal);
    outline: none;
  }
  /* Three columns need room. A fixed 260px sidebar plus a 340px minimum pane
     left the stream about 150 pixels wide -- narrow enough that its composer
     could not be typed in -- so the layout gives way in stages rather than
     squeezing everything equally. */
  @media (max-width: 1180px) {
    .shell,
    .shell.threaded {
      grid-template-columns: 210px minmax(0, 1fr);
    }
    .shell.threaded {
      grid-template-columns: 210px minmax(0, 1fr) 6px var(--thread-width);
    }
  }
  @media (max-width: 900px) {
    /* Below this there is no honest way to show both: the thread takes the
       stream's place, and closing it brings the stream back. Nothing to drag
       either, so the handle goes with it. */
    .shell.threaded {
      grid-template-columns: 210px minmax(0, 1fr);
    }
    .shell.threaded > main,
    .shell.threaded > .divider {
      display: none;
    }
  }
  /* The sidebar; the thread pane brings its own frame. */
  .shell > aside:first-child {
    border-right: 1px solid var(--rule);
    background: var(--surface);
    /* The sidebar itself no longer scrolls: the channel list does. A rail
       inside a scrolling sidebar would scroll away from the thing it is for. */
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .sidebar-body {
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .rail {
    display: flex;
    flex-direction: column;
    flex: none;
    gap: 4px;
    padding: 8px 4px;
    border-right: 1px solid var(--rule-soft, var(--rule));
    overflow-y: auto;
    scrollbar-width: none;
  }
  .rail::-webkit-scrollbar {
    width: 0;
  }
  .rail button {
    position: relative;
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    padding: 0;
    border: 1px solid transparent;
    border-radius: 7px;
    background: var(--ground);
    color: var(--ink-soft);
    font: inherit;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
  }
  .rail button:hover {
    border-color: var(--rule);
    color: var(--ink);
  }
  /* Where you are, not what you last clicked: it follows the open channel's
     team, so it stays true when a channel is reached from search or a mention. */
  .rail button.current {
    border-color: var(--signal);
    color: var(--ink);
  }
  .rail img {
    width: 100%;
    height: 100%;
    border-radius: 6px;
    object-fit: cover;
  }
  /* Behind the icon, so a team with no icon on the server still reads as
     something rather than as an empty square. */
  .rail .initial {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    z-index: -1;
  }
  .rail .pip {
    position: absolute;
    top: -3px;
    right: -3px;
    min-width: 8px;
    height: 8px;
    border-radius: 5px;
    background: var(--ink-faint);
    box-shadow: 0 0 0 2px var(--surface);
  }
  .rail .pip.mention {
    height: 14px;
    min-width: 14px;
    padding: 0 3px;
    font-size: 9.5px;
    line-height: 14px;
    text-align: center;
    color: var(--ground);
    background: var(--flag);
  }
  .who {
    display: flex;
    align-items: center;
    gap: 8px;
    justify-content: space-between;
    padding: 12px 14px;
    border-bottom: 1px solid var(--rule);
    /* The status picker hangs off this bar. */
    position: relative;
  }
  .badge {
    font-family: var(--mono);
    font-size: 10px;
    text-transform: uppercase;
    color: var(--ink-faint);
  }
  .badge.live {
    color: var(--signal);
  }
  nav {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 6px;
    gap: 1px;
  }
  nav button {
    display: flex;
    align-items: center;
    gap: 7px;
    background: none;
    border: 0;
    /* A read channel recedes. This is half of what makes an unread one stand
       out: `color: inherit` put both on the same colour, leaving font weight as
       the only difference between them -- which at 13px is nearly nothing. */
    color: var(--ink-soft);
    font: inherit;
    font-size: 13px;
    text-align: left;
    padding: 5px 8px;
    border-radius: 4px;
    cursor: pointer;
  }
  /* The name is the flexible part: the icon keeps its size and the unread
     count keeps its place, so a long channel name is what gets truncated. */
  nav button .name { flex: 1; min-width: 0; }
  nav button .count { flex: none; }
  nav button:hover {
    background: var(--surface-2);
  }
  nav button.selected {
    background: var(--signal-soft);
    color: var(--signal);
  }
  /* A muted channel is dimmed whole, count included. It still says how much is
     there -- muting is "do not interrupt me", not "hide this from me" -- but it
     stops competing with channels that do want an answer, and it matches the
     taskbar badge, which excludes muted channels entirely. */
  nav button.muted .name,
  nav button.muted .count {
    opacity: 0.45;
  }
  nav button.muted .count.mention {
    /* Not the signal colour: a mention in a muted channel was still asked for
       quietly. */
    background: var(--surface-2);
    color: var(--ink-faint);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Unread: full contrast and properly bold, icon included. */
  nav button.unread {
    color: var(--ink);
  }
  nav button.unread .name {
    font-weight: 700;
  }
  nav button.unread .count {
    color: var(--ink);
    font-weight: 700;
  }
  /* Selection outranks unread: it is where the reader actually is, and the two
     rules would otherwise be decided by source order alone. */
  nav button.selected.unread {
    color: var(--signal);
  }
  /* A team heading: the outer grouping, and only drawn when there are two.
     Deliberately unlike a category rather than a smaller version of one -- it
     used to be 10.5px faint uppercase against a category's 11.5px faint
     uppercase, so the container read as *quieter* than its contents and teams
     were lost in the list. Bigger, full contrast, title case, and a rule above
     it: the name of a place, not another label. */
  .team-icon {
    width: 18px;
    height: 18px;
    border-radius: 4px;
    object-fit: cover;
  }
  .team .add {
    flex: none;
    font: inherit;
    font-size: 13px;
    line-height: 1;
    padding: 0 2px;
    border: 0;
    background: none;
    color: var(--ink-faint);
    cursor: pointer;
  }
  .team .add:hover {
    color: var(--ink);
  }
  .team span {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .team {
    display: flex;
    align-items: center;
    gap: 7px;
    margin: 18px 12px 4px;
    padding-top: 12px;
    border-top: 1px solid var(--rule);
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 0.01em;
    color: var(--ink);
  }
  /* The first one has nothing above it to be separated from. */
  .team:first-child {
    margin-top: 6px;
    padding-top: 0;
    border-top: 0;
  }
  .group-row {
    display: flex;
    align-items: center;
  }
  .group-row .add {
    flex: none;
    font: inherit;
    font-size: 13px;
    line-height: 1;
    padding: 2px 10px 2px 4px;
    border: 0;
    background: none;
    color: var(--ink-faint);
    cursor: pointer;
  }
  .group-row .add:hover {
    color: var(--ink);
  }
  nav button.group {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 3px 12px;
    border: 0;
    background: transparent;
    color: var(--ink-faint);
    font: inherit;
    font-size: 11.5px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    cursor: pointer;
  }
  nav button.group:hover { color: var(--ink); }
  /* The unread section leads the sidebar, so it reads as the thing to act on
     rather than as another category. */
  nav button.group.unreads { color: var(--ink); font-weight: 600; }
  nav button.group .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .twist {
    display: inline-block;
    transition: transform 120ms ease;
  }
  .twist.open { transform: rotate(90deg); }
  .count {
    font-family: var(--mono);
    font-size: 10.5px;
    background: var(--surface-2);
    border-radius: 9px;
    padding: 0 6px;
  }
  .count.mention {
    background: var(--signal);
    color: var(--surface);
  }
  main {
    display: grid;
    /* Header, stream, typing line, composer. */
    grid-template-rows: auto 1fr auto auto;
    /* And one column, held to the width of this element. Named rather than
       left implicit: an implicit column is `auto`, which sizes to its widest
       child's min-content -- measured at 921px against a 698px main with a
       thread open, so the header, stream, typing line and composer all painted
       223px over the divider and into the thread pane. */
    grid-template-columns: minmax(0, 1fr);
    min-height: 0;
    min-width: 0;
    /* The jump-to-newest button floats over the stream. */
    position: relative;
  }
  .channel {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--rule);
    background: var(--surface);
    /* The options panel hangs off this header. */
    position: relative;
  }
  .channel strong {
    font-size: 14px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .muted-note {
    font-size: 11px;
    color: var(--ink-faint);
    border: 1px solid var(--rule);
    border-radius: 8px;
    padding: 0 6px;
  }
  .scroll-frame {
    position: relative;
    display: grid;
    min-height: 0;
  }
  .scroll {
    overflow-y: auto;
    min-height: 0;
    /* The native bar is hidden, not absent: the element still scrolls, and
       `Scrollbar` draws the thumb. See that component for why. */
    scrollbar-width: none;
    /* The browser's own scroll anchoring, off.
     *
     * It keeps a chosen element still whenever content above it resizes -- and
     * a virtualiser resizes the content above the viewport on every frame, by
     * design, as its spacers grow and shrink. This list does its own
     * anchoring, through `onlayout`, where it can tell a reader's drag from a
     * height correction. */
    overflow-anchor: none;
  }
  /* Laid out but not painted while the opening scroll finds its position. See
     `opening` in the script for why it is `visibility`. */
  .scroll.opening {
    visibility: hidden;
  }
  .scroll::-webkit-scrollbar {
    width: 0;
    height: 0;
  }
  .centre {
    display: grid;
    place-items: center;
    height: 100vh;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 320px;
    background: var(--surface);
    border: 1px solid var(--rule);
    border-radius: 6px;
    padding: 22px;
  }
  h1 {
    margin: 0;
    font-size: 20px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12.5px;
    color: var(--ink-soft);
  }
  input {
    font: inherit;
    padding: 6px 8px;
    border: 1px solid var(--rule);
    border-radius: 4px;
    background: var(--ground);
    color: var(--ink);
  }
  button[type="submit"] {
    font: inherit;
    padding: 7px;
    border: 0;
    border-radius: 4px;
    background: var(--signal);
    color: #fff;
    cursor: pointer;
  }
  .hint {
    color: var(--ink-soft);
    font-size: 12.5px;
    margin: 0;
  }
  .own-status {
    display: flex;
    align-items: center;
    gap: 6px;
    font: inherit;
    font-size: 11px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 3px 7px;
    border: 1px solid transparent;
    border-radius: 10px;
    background: none;
    color: var(--ink-faint);
    cursor: pointer;
  }
  .own-status:hover,
  .own-status[aria-expanded="true"] {
    border-color: var(--rule);
    color: var(--ink);
  }
  .own-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--ink-faint, #6d7d8c);
  }
  .own-status.online .own-dot,
  .own-dot.online {
    background: var(--ok, #4ec49b);
  }
  .own-status.away .own-dot,
  .own-dot.away {
    background: var(--flag, #e0a13f);
  }
  .own-status.dnd .own-dot,
  .own-dot.dnd {
    background: #c0392b;
  }
  .status-picker {
    position: absolute;
    top: 100%;
    left: 10px;
    z-index: 30;
    display: flex;
    flex-direction: column;
    min-width: 170px;
    padding: 4px;
    border: 1px solid var(--rule);
    border-radius: 6px;
    background: var(--surface);
    box-shadow: 0 6px 18px rgb(0 0 0 / 0.22);
  }
  .status-picker button {
    display: flex;
    align-items: center;
    gap: 8px;
    font: inherit;
    font-size: 12.5px;
    text-align: left;
    padding: 5px 8px;
    border: 0;
    border-radius: 4px;
    background: none;
    color: var(--ink);
    cursor: pointer;
  }
  .status-picker button:hover {
    background: var(--ground);
  }
  .status-picker button.current {
    color: var(--signal);
    font-weight: 600;
  }
  .search-field {
    margin-left: auto;
    width: clamp(140px, 22vw, 260px);
    font: inherit;
    font-size: 12.5px;
    padding: 5px 9px;
    border: 1px solid var(--rule);
    border-radius: 14px;
    background: var(--ground);
    color: var(--ink);
  }
  .search-field:focus-visible {
    outline: 2px solid var(--signal);
    outline-offset: -1px;
  }
  .options-button {
    font: inherit;
    font-size: 13px;
    line-height: 1;
    padding: 5px 7px;
    border: 1px solid transparent;
    border-radius: 5px;
    background: none;
    color: var(--ink-soft);
    cursor: pointer;
  }
  .options-button:hover,
  .options-button[aria-expanded="true"] {
    border-color: var(--rule);
    background: var(--ground);
  }
  .options {
    position: absolute;
    top: 100%;
    right: 10px;
    z-index: 20;
    width: 300px;
    padding: 10px 12px;
    border: 1px solid var(--rule);
    border-radius: 6px;
    background: var(--surface);
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.18);
  }
  .options label {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 13px;
  }
  .options p {
    margin: 6px 0 0;
    font-size: 11.5px;
    line-height: 1.45;
    color: var(--ink-soft);
  }
  .scroll.dropping {
    outline: 2px dashed var(--signal);
    outline-offset: -4px;
  }
  .history {
    position: absolute;
    top: 8px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 5;
    margin: 0;
    font-size: 11.5px;
    letter-spacing: 0.04em;
    padding: 4px 12px;
    border: 1px solid var(--rule);
    border-radius: 12px;
    background: var(--surface);
    color: var(--ink-faint);
    box-shadow: 0 3px 10px rgb(0 0 0 / 0.22);
    pointer-events: none;
  }
  .trail {
    display: inline-flex;
    gap: 2px;
    margin-right: 2px;
  }
  .step {
    font: inherit;
    font-size: 15px;
    line-height: 1;
    width: 22px;
    padding: 3px 0;
    border: 1px solid transparent;
    border-radius: 5px;
    background: none;
    color: var(--ink-faint);
    cursor: pointer;
  }
  .step:hover:not(:disabled) {
    border-color: var(--rule);
    color: var(--ink);
  }
  .step:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .leave,
  .confirm-leave button {
    font: inherit;
    font-size: 12.5px;
    padding: 4px 8px;
    border: 1px solid var(--rule);
    border-radius: 5px;
    background: var(--ground);
    color: var(--ink);
    cursor: pointer;
  }
  .leave {
    align-self: flex-start;
    color: var(--flag);
  }
  .confirm-leave {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: 12.5px;
  }
  .confirm-leave .danger {
    border-color: var(--flag);
    color: var(--flag);
  }
  .to-newest {
    position: absolute;
    left: 50%;
    transform: translateX(-50%);
    bottom: 86px;
    z-index: 5;
    font: inherit;
    font-size: 12px;
    padding: 6px 12px;
    border: 1px solid var(--rule);
    border-radius: 14px;
    background: var(--surface);
    color: var(--ink);
    box-shadow: 0 3px 10px rgb(0 0 0 / 0.22);
    cursor: pointer;
  }
  .to-newest:hover {
    border-color: var(--signal);
    color: var(--signal);
  }
  .typing {
    font-size: 12px;
    line-height: 16px;
    color: var(--ink-faint);
    padding: 0 16px 10px;
    margin: 0;
    /* Fixed, so an arriving indicator does not shift the conversation. */
    height: 16px;
    box-sizing: content-box;
  }
  /* A button that has to read as part of the sentence around it. */
  .linkish {
    font: inherit;
    font-size: inherit;
    padding: 0;
    border: 0;
    background: none;
    color: var(--signal);
    text-decoration: underline;
    cursor: pointer;
  }
  .error {
    color: var(--flag);
    font-size: 12.5px;
    padding: 0 16px;
  }
</style>
