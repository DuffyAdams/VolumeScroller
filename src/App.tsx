import { type CSSProperties, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Icon } from "@iconify/react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { volumeIcons } from "./hugeiconsVolume";

type VolumePayload = {
  volume: number;
  muted: boolean;
  direction: number;
};

type Preferences = {
  scrollerEnabled: boolean;
  launchAtStartup: boolean;
  startMinimizedToTray: boolean;
  scrollIncrement: number;
  scrollDirection: "upIncreases" | "downIncreases";
  pauseWhileHovering: boolean;
  pauseInFullscreenApps: boolean;
  showTrayIcon: boolean;
  checkForUpdatesAutomatically: boolean;
  overlayWidth: number;
  overlayHeight: number;
  horizontalPosition: "left" | "center" | "right";
  verticalPosition: "top" | "center" | "bottom";
  horizontalOffset: number;
  verticalOffset: number;
  theme: ThemeName;
};

const themeOptions = [
  { value: "monochrome", label: "Monochrome" },
  { value: "windows11", label: "Windows 11" },
  { value: "ubuntu", label: "Ubuntu" },
  { value: "solarized", label: "Solarized" }
] as const;

type ThemeName = (typeof themeOptions)[number]["value"];

const isTauri = "__TAURI_INTERNALS__" in window;
const currentWindowLabel = isTauri ? getCurrentWindow().label : "main";
const isSettingsWindow = currentWindowLabel === "settings";
const segmentCount = 12;
const taskbarIconSrc = "/taskbar-icon.ico";
const defaultPreferences: Preferences = {
  scrollerEnabled: true,
  launchAtStartup: false,
  startMinimizedToTray: true,
  scrollIncrement: 3.5,
  scrollDirection: "upIncreases",
  pauseWhileHovering: true,
  pauseInFullscreenApps: true,
  showTrayIcon: true,
  checkForUpdatesAutomatically: true,
  overlayWidth: 150,
  overlayHeight: 40,
  horizontalPosition: "center",
  verticalPosition: "bottom",
  horizontalOffset: 0,
  verticalOffset: 76,
  theme: "monochrome"
};
const overlayScaleMin = 75;
const overlayScaleMax = 210;

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function pixelSnappedVars(preferences: Preferences, ratio = window.devicePixelRatio || 1) {
  const scale = clamp(Math.min(preferences.overlayWidth / 150, preferences.overlayHeight / 40), 0.78, 2.2);
  const cardWidth = Math.max(82, preferences.overlayWidth - 28);
  const cardHeight = Math.max(22, preferences.overlayHeight - 16);
  const cardPaddingY = Math.max(3, 4 * scale);
  const segmentBarHeight = Math.max(11, cardHeight - cardPaddingY * 2 - 4);
  const vars: Record<string, number | string> = {
    width: `${preferences.overlayWidth}px`,
    height: `${preferences.overlayHeight}px`,
    "--shell-transition-y": preferences.verticalPosition === "top" ? -18 : 18,
    "--shell-padding": 5,
    "--card-width": cardWidth,
    "--card-height": cardHeight,
    "--icon-column": Math.max(14, 15 * scale),
    "--percent-column": Math.max(22, 22 * scale),
    "--card-gap": Math.max(3, 4 * scale),
    "--card-padding-y": cardPaddingY,
    "--card-padding-x": Math.max(5, 6 * scale),
    "--icon-size": Math.max(13, 14 * scale),
    "--segment-gap": Math.max(1, 2 * scale),
    "--segment-height": segmentBarHeight,
    "--segment-bar-height": segmentBarHeight,
    "--percentage-size": Math.max(8, 8 * scale)
  };

  for (const [name, value] of Object.entries(vars)) {
    if (typeof value === "string") {
      continue;
    }

    const snappedValue = Math.round(value * ratio) / ratio;
    vars[name] = `${snappedValue}px`;
  }

  return vars as CSSProperties;
}

function syncPixelSnappedVars(preferences: Preferences) {
  const vars = pixelSnappedVars(preferences);

  for (const [name, value] of Object.entries(vars)) {
    if (name.startsWith("--")) {
      document.documentElement.style.setProperty(name, String(value));
    }
  }
}

function normalizePreferences(preferences: Preferences): Preferences {
  return {
    scrollerEnabled: preferences.scrollerEnabled !== false,
    launchAtStartup: preferences.launchAtStartup === true,
    startMinimizedToTray: preferences.startMinimizedToTray !== false,
    scrollIncrement: clamp(Number(preferences.scrollIncrement) || 3.5, 0.5, 25),
    scrollDirection: isScrollDirection(preferences.scrollDirection) ? preferences.scrollDirection : "upIncreases",
    pauseWhileHovering: preferences.pauseWhileHovering !== false,
    pauseInFullscreenApps: preferences.pauseInFullscreenApps !== false,
    showTrayIcon: preferences.showTrayIcon !== false,
    checkForUpdatesAutomatically: preferences.checkForUpdatesAutomatically !== false,
    overlayWidth: clamp(Number(preferences.overlayWidth) || 150, 96, 320),
    overlayHeight: clamp(Number(preferences.overlayHeight) || 40, 30, 120),
    horizontalPosition: preferences.horizontalPosition,
    verticalPosition: preferences.verticalPosition,
    horizontalOffset: clamp(Math.round(Number(preferences.horizontalOffset) || 0), -400, 400),
    verticalOffset: clamp(Math.round(Number(preferences.verticalOffset) || 0), -400, 400),
    theme: isThemeName(preferences.theme) ? preferences.theme : "monochrome"
  };
}

function isScrollDirection(value: string): value is Preferences["scrollDirection"] {
  return value === "upIncreases" || value === "downIncreases";
}

function isThemeName(value: string): value is ThemeName {
  return themeOptions.some((option) => option.value === value);
}

function linkedOverlayScale(preferences: Preferences) {
  const widthScale = preferences.overlayWidth / defaultPreferences.overlayWidth;
  const heightScale = preferences.overlayHeight / defaultPreferences.overlayHeight;
  return clamp(Math.round(((widthScale + heightScale) / 2) * 100), overlayScaleMin, overlayScaleMax);
}

function App() {
  const [preferences, setPreferences] = useState(defaultPreferences);

  useEffect(() => {
    if (!isTauri) {
      return;
    }

    let unlistenPreferences = () => {};
    let unlistenPreview = () => {};

    invoke<Preferences>("get_preferences")
      .then((nextPreferences) => setPreferences(normalizePreferences(nextPreferences)))
      .catch(() => undefined);

    listen<Preferences>("preferences-changed", (event) => {
      setPreferences(normalizePreferences(event.payload));
    })
      .then((cleanup) => {
        unlistenPreferences = cleanup;
      })
      .catch(() => undefined);

    listen<Preferences>("preferences-preview", (event) => {
      if (!isSettingsWindow) {
        setPreferences(normalizePreferences(event.payload));
      }
    })
      .then((cleanup) => {
        unlistenPreview = cleanup;
      })
      .catch(() => undefined);

    return () => {
      unlistenPreferences();
      unlistenPreview();
    };
  }, []);

  if (isSettingsWindow) {
    return <SettingsView preferences={preferences} setPreferences={setPreferences} />;
  }

  return <OverlayView preferences={preferences} />;
}

function OverlayView({ preferences }: { preferences: Preferences }) {
  const [volume, setVolume] = useState(42);
  const [muted, setMuted] = useState(false);
  const [direction, setDirection] = useState(0);
  const [overlayPhase, setOverlayPhase] = useState<"hidden" | "visible" | "leaving">(!isTauri ? "visible" : "hidden");
  const [animationTick, setAnimationTick] = useState(0);
  const volumeRef = useRef(volume);
  const hideTimer = useRef<number>();
  const exitTimer = useRef<number>();
  const visible = overlayPhase === "visible";

  const activeSegments = muted ? 0 : Math.round((volume / 100) * segmentCount);
  const volumeIcon = muted || volume === 0 ? volumeIcons.muted : volume < 55 ? volumeIcons.low : volumeIcons.high;

  function finishHide() {
    setOverlayPhase(isTauri ? "hidden" : "visible");
  }

  function scheduleHide() {
    window.clearTimeout(hideTimer.current);
    window.clearTimeout(exitTimer.current);

    hideTimer.current = window.setTimeout(() => {
      if (!isTauri) {
        return;
      }

      setOverlayPhase("leaving");
      exitTimer.current = window.setTimeout(finishHide, 380);
    }, 1450);
  }

  function reveal(payload: VolumePayload) {
    window.clearTimeout(hideTimer.current);
    window.clearTimeout(exitTimer.current);
    const nextVolume = clamp(Math.round(payload.volume), 0, 100);
    volumeRef.current = nextVolume;
    setVolume(nextVolume);
    setMuted(payload.muted);
    setDirection(payload.direction);
    setAnimationTick((current) => current + 1);
    setOverlayPhase("visible");
    scheduleHide();
  }

  useLayoutEffect(() => {
    syncPixelSnappedVars(preferences);

    const onResize = () => syncPixelSnappedVars(preferences);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [preferences]);

  useEffect(() => {
    if (isTauri) {
      return;
    }

    const onWheel = (event: WheelEvent) => {
      const amount = preferences.scrollIncrement;
      const rawDirection = event.deltaY < 0 ? 1 : -1;
      const direction = preferences.scrollDirection === "upIncreases" ? rawDirection : -rawDirection;
      const next = clamp(volumeRef.current + direction * amount, 0, 100);
      reveal({ volume: next, muted: next === 0, direction });
    };

    window.addEventListener("wheel", onWheel, { passive: true });
    return () => window.removeEventListener("wheel", onWheel);
  }, [preferences.scrollDirection, preferences.scrollIncrement]);

  useEffect(() => {
    if (!isTauri) {
      return;
    }

    let unlisten = () => {};

    invoke<VolumePayload>("get_volume")
      .then(reveal)
      .catch(() => undefined);

    listen<VolumePayload>("volume-changed", (event) => reveal(event.payload))
      .then((cleanup) => {
        unlisten = cleanup;
      })
      .catch(() => undefined);

    return () => {
      window.clearTimeout(hideTimer.current);
      window.clearTimeout(exitTimer.current);
      unlisten();
    };
  }, []);

  function pauseHideTimer() {
    if (!preferences.pauseWhileHovering) {
      return;
    }

    window.clearTimeout(hideTimer.current);
    window.clearTimeout(exitTimer.current);
  }

  function resumeHideTimer() {
    if (!preferences.pauseWhileHovering || !visible) {
      return;
    }

    scheduleHide();
  }

  return (
    <main
      className={`shell theme-${preferences.theme} ${overlayPhase === "visible" ? "is-visible" : ""} ${
        overlayPhase === "leaving" ? "is-leaving" : ""
      }`}
      onMouseEnter={pauseHideTimer}
      onMouseLeave={resumeHideTimer}
    >
      <section className={`volume-card ${muted ? "is-muted" : ""}`}>
        <div className="volume-icon" aria-hidden="true">
          <Icon icon={volumeIcon} width="100%" height="100%" />
        </div>
        <div
          key={animationTick}
          className={`segments ${direction > 0 ? "nudging-up" : direction < 0 ? "nudging-down" : ""}`}
          aria-hidden="true"
        >
          {Array.from({ length: segmentCount }, (_, index) => (
            <span
              key={index}
              className={`${index < activeSegments ? "is-active" : ""} ${
                index >= segmentCount - 2 ? "is-danger" : ""
              }`}
              style={{ transitionDelay: `${index * 10}ms` }}
            />
          ))}
        </div>
        <strong className="percentage">{muted ? "0%" : `${volume}%`}</strong>
      </section>
    </main>
  );
}

function SettingsView({
  preferences,
  setPreferences
}: {
  preferences: Preferences;
  setPreferences: (preferences: Preferences) => void;
}) {
  const [draft, setDraft] = useState(preferences);
  const [draftScale, setDraftScale] = useState(linkedOverlayScale(preferences));

  useEffect(() => {
    setDraft(preferences);
    setDraftScale(linkedOverlayScale(preferences));
  }, [preferences]);

  useEffect(() => {
    if (!isTauri) {
      return;
    }

    invoke("preview_preferences", { preferences: normalizePreferences(draft) }).catch(() => undefined);
  }, [draft]);

  function patchDraft(patch: Partial<Preferences>) {
    setDraft((current) => normalizePreferences({ ...current, ...patch }));
  }

  function resetDraftValue(key: keyof Preferences) {
    patchDraft({ [key]: defaultPreferences[key] } as Partial<Preferences>);
  }

  function patchDraftOverlayScale(scale: number) {
    setDraftScale(scale);
    patchDraft({
      overlayWidth: (defaultPreferences.overlayWidth * scale) / 100,
      overlayHeight: (defaultPreferences.overlayHeight * scale) / 100
    });
  }

  function resetDraftOverlayScale() {
    setDraftScale(100);
    patchDraft({
      overlayWidth: defaultPreferences.overlayWidth,
      overlayHeight: defaultPreferences.overlayHeight
    });
  }

  async function save() {
    try {
      const saved = await invoke<Preferences>("save_preferences", { preferences: normalizePreferences(draft) });
      setPreferences(normalizePreferences(saved));
    } catch {
      undefined;
    }
  }

  async function reset() {
    try {
      const saved = await invoke<Preferences>("reset_preferences");
      setPreferences(normalizePreferences(saved));
    } catch {
      undefined;
    }
  }

  const scaleIsDefault = draftScale === 100;

  return (
    <main className={`settings-app theme-${draft.theme}`}>
      <div className="settings-pane">
        <header className="settings-header">
          <div>
            <span className="eyebrow">Settings</span>
            <h1>Volume Scroller</h1>
          </div>
          <div className="settings-header-icon" aria-hidden="true">
            <img src={taskbarIconSrc} alt="" />
          </div>
        </header>

        <section className="settings-section" id="behavior">
          <h2>Behavior</h2>
          <div className="settings-group">
            <ToggleRow
              label="Enable Taskbar Scroller"
              checked={draft.scrollerEnabled}
              onChange={(scrollerEnabled) => patchDraft({ scrollerEnabled })}
            />
            <ToggleRow
              label="Launch at Windows startup"
              checked={draft.launchAtStartup}
              onChange={(launchAtStartup) => patchDraft({ launchAtStartup })}
            />
            <ToggleRow
              label="Start minimized to tray"
              checked={draft.startMinimizedToTray}
              onChange={(startMinimizedToTray) => patchDraft({ startMinimizedToTray })}
            />
            <ToggleRow
              label="Pause while hovering"
              checked={draft.pauseWhileHovering}
              onChange={(pauseWhileHovering) => patchDraft({ pauseWhileHovering })}
            />
            <ToggleRow
              label="Pause in fullscreen apps"
              checked={draft.pauseInFullscreenApps}
              onChange={(pauseInFullscreenApps) => patchDraft({ pauseInFullscreenApps })}
            />
            <ToggleRow
              label="Show tray icon"
              checked={draft.showTrayIcon}
              onChange={(showTrayIcon) => patchDraft({ showTrayIcon })}
            />
            <ToggleRow
              label="Check for updates automatically"
              checked={draft.checkForUpdatesAutomatically}
              onChange={(checkForUpdatesAutomatically) => patchDraft({ checkForUpdatesAutomatically })}
            />
          </div>
        </section>

        <section className="settings-section" id="preview">
          <h2>Preview</h2>
          <div className="settings-group">
            <div className="preview-row">
              <span>Full Bar</span>
              <div className="volume-preview-area">
                <VolumePreview preferences={draft} />
              </div>
            </div>
          </div>
        </section>

        <section className="settings-section" id="scroll">
          <h2>Scroll</h2>
          <div className="settings-group">
            <div className="segmented-row">
              <span>Direction</span>
              <Segmented
                value={draft.scrollDirection}
                options={["upIncreases", "downIncreases"]}
                labels={{ upIncreases: "Up increases", downIncreases: "Down increases" }}
                onChange={(value) => patchDraft({ scrollDirection: value as Preferences["scrollDirection"] })}
              />
            </div>
            <label className="control-row">
              <span>Speed</span>
              <output>{draft.scrollIncrement.toFixed(1)}%</output>
              <input
                type="range"
                min="0.5"
                max="25"
                step="0.5"
                value={draft.scrollIncrement}
                onChange={(event) => patchDraft({ scrollIncrement: Number(event.currentTarget.value) })}
              />
            </label>
          </div>
        </section>

        <section className="settings-section" id="size">
          <h2>Size</h2>
          <div className="settings-group">
            <div className="control-row">
              <span>Scale</span>
              <div className="control-meta">
                <output id="overlay-scale-value">{draftScale}%</output>
                <ResetIconButton
                  label="Reset scale"
                  disabled={scaleIsDefault}
                  onClick={resetDraftOverlayScale}
                />
              </div>
              <input
                aria-label="Scale width and height"
                aria-describedby="overlay-scale-value"
                type="range"
                min={overlayScaleMin}
                max={overlayScaleMax}
                step="5"
                value={draftScale}
                onChange={(event) => patchDraftOverlayScale(Number(event.currentTarget.value))}
              />
            </div>
            <div className="control-row">
              <span>Width</span>
              <div className="control-meta">
                <output id="overlay-width-value">{Math.round(draft.overlayWidth)}px</output>
                <ResetIconButton
                  label="Reset width"
                  disabled={draft.overlayWidth === defaultPreferences.overlayWidth}
                  onClick={() => resetDraftValue("overlayWidth")}
                />
              </div>
              <input
                aria-label="Width"
                aria-describedby="overlay-width-value"
                type="range"
                min="96"
                max="320"
                step="2"
                value={draft.overlayWidth}
                onChange={(event) => patchDraft({ overlayWidth: Number(event.currentTarget.value) })}
              />
            </div>
            <div className="control-row">
              <span>Height</span>
              <div className="control-meta">
                <output id="overlay-height-value">{Math.round(draft.overlayHeight)}px</output>
                <ResetIconButton
                  label="Reset height"
                  disabled={draft.overlayHeight === defaultPreferences.overlayHeight}
                  onClick={() => resetDraftValue("overlayHeight")}
                />
              </div>
              <input
                aria-label="Height"
                aria-describedby="overlay-height-value"
                type="range"
                min="30"
                max="120"
                step="2"
                value={draft.overlayHeight}
                onChange={(event) => patchDraft({ overlayHeight: Number(event.currentTarget.value) })}
              />
            </div>
          </div>
        </section>

        <section className="settings-section" id="position">
          <h2>Position</h2>
          <div className="settings-group">
            <div className="segmented-row">
              <span>Horizontal</span>
              <Segmented
                value={draft.horizontalPosition}
                options={["left", "center", "right"]}
                onChange={(value) => patchDraft({ horizontalPosition: value as Preferences["horizontalPosition"] })}
              />
            </div>
            <div className="segmented-row">
              <span>Vertical</span>
              <Segmented
                value={draft.verticalPosition}
                options={["top", "center", "bottom"]}
                onChange={(value) => patchDraft({ verticalPosition: value as Preferences["verticalPosition"] })}
              />
            </div>
            <div className="control-row">
              <span>X offset</span>
              <div className="control-meta">
                <output id="horizontal-offset-value">{draft.horizontalOffset}px</output>
                <ResetIconButton
                  label="Reset X offset"
                  disabled={draft.horizontalOffset === defaultPreferences.horizontalOffset}
                  onClick={() => resetDraftValue("horizontalOffset")}
                />
              </div>
              <input
                aria-label="X offset"
                aria-describedby="horizontal-offset-value"
                type="range"
                min="-400"
                max="400"
                step="4"
                value={draft.horizontalOffset}
                onChange={(event) => patchDraft({ horizontalOffset: Number(event.currentTarget.value) })}
              />
            </div>
            <div className="control-row">
              <span>Y offset</span>
              <div className="control-meta">
                <output id="vertical-offset-value">{draft.verticalOffset}px</output>
                <ResetIconButton
                  label="Reset Y offset"
                  disabled={draft.verticalOffset === defaultPreferences.verticalOffset}
                  onClick={() => resetDraftValue("verticalOffset")}
                />
              </div>
              <input
                aria-label="Y offset"
                aria-describedby="vertical-offset-value"
                type="range"
                min="-400"
                max="400"
                step="4"
                value={draft.verticalOffset}
                onChange={(event) => patchDraft({ verticalOffset: Number(event.currentTarget.value) })}
              />
            </div>
          </div>
        </section>

        <section className="settings-section" id="theme">
          <h2>Theme</h2>
          <div className="settings-group">
            <div className="theme-row">
              <span>Palette</span>
              <ThemePicker value={draft.theme} onChange={(theme) => patchDraft({ theme })} />
            </div>
          </div>
        </section>

        <footer className="settings-actions">
          <button type="button" className="secondary-button" onClick={reset}>
            Reset
          </button>
          <button type="button" className="primary-button" onClick={save}>
            Save
          </button>
        </footer>
      </div>
    </main>
  );
}

function ResetIconButton({
  label,
  disabled,
  onClick
}: {
  label: string;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <button type="button" className="reset-icon-button" aria-label={label} title={label} disabled={disabled} onClick={onClick}>
      <Icon icon={volumeIcons.reset} width="16" height="16" />
    </button>
  );
}

function Segmented({
  value,
  options,
  labels,
  onChange
}: {
  value: string;
  options: string[];
  labels?: Record<string, string>;
  onChange: (value: string) => void;
}) {
  return (
    <div
      className="segmented"
      role="group"
      style={{ gridTemplateColumns: `repeat(${options.length}, minmax(0, 1fr))` }}
    >
      {options.map((option) => (
        <button
          key={option}
          type="button"
          className={option === value ? "is-selected" : ""}
          onClick={() => onChange(option)}
        >
          {labels?.[option] ?? option}
        </button>
      ))}
    </div>
  );
}

function ToggleRow({
  label,
  checked,
  onChange
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="toggle-row">
      <span>{label}</span>
      <input type="checkbox" checked={checked} onChange={(event) => onChange(event.currentTarget.checked)} />
    </label>
  );
}

function VolumePreview({ preferences }: { preferences: Preferences }) {
  return (
    <div className={`volume-preview-shell theme-${preferences.theme}`} style={pixelSnappedVars(preferences, 1)}>
      <section className="volume-card">
        <div className="volume-icon" aria-hidden="true">
          <Icon icon={volumeIcons.high} width="100%" height="100%" />
        </div>
        <div className="segments" aria-hidden="true">
          {Array.from({ length: segmentCount }, (_, index) => (
            <span key={index} className={`is-active ${index >= segmentCount - 2 ? "is-danger" : ""}`} />
          ))}
        </div>
        <strong className="percentage">100%</strong>
      </section>
    </div>
  );
}

function ThemePicker({ value, onChange }: { value: ThemeName; onChange: (value: ThemeName) => void }) {
  return (
    <div className="theme-picker" role="radiogroup" aria-label="Theme palette">
      {themeOptions.map((option) => (
        <button
          key={option.value}
          type="button"
          className={`theme-choice theme-swatch-${option.value} ${option.value === value ? "is-selected" : ""}`}
          role="radio"
          aria-checked={option.value === value}
          onClick={() => onChange(option.value)}
        >
          <span className="theme-swatch" aria-hidden="true">
            <span />
            <span />
            <span />
          </span>
          <span>{option.label}</span>
        </button>
      ))}
    </div>
  );
}

export default App;
