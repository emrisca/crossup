import { useEffect, useState } from "react";
import { ArrowUp } from "lucide-react";
import { isTauri, updateInstall, updateState, type UpdateView } from "@/bridge";

/** The shell checks on its own clock; this only reads the state it keeps. */
const POLL_MS = 4000;

const megabytes = (bytes: number) => `${(bytes / (1024 * 1024)).toFixed(1)} MB`;

/**
 * The one row under the header that appears when a newer release is waiting.
 *
 * Nothing installs without the click. Later hides that version for the rest of
 * the run, and a failed check draws nothing: an update nobody asked about must
 * not become a warning on every launch.
 */
export function UpdateBanner() {
  const [update, setUpdate] = useState<UpdateView | null>(null);
  /** The version somebody chose to skip until the next launch. */
  const [later, setLater] = useState<string | null>(null);

  useEffect(() => {
    if (!isTauri()) return;
    let live = true;
    const read = async () => {
      const next = await updateState().catch(() => null);
      if (live && next) setUpdate(next);
    };
    void read();
    const id = window.setInterval(read, POLL_MS);
    return () => {
      live = false;
      window.clearInterval(id);
    };
  }, []);

  if (!update || !update.version) return null;

  const working =
    update.status === "downloading" ||
    update.status === "installing" ||
    update.status === "restarting";
  if (update.status !== "available" && !working) return null;
  if (later === update.version) return null;

  const progress = update.total
    ? `${megabytes(update.downloaded)} of ${megabytes(update.total)}`
    : megabytes(update.downloaded);

  return (
    <div className="hb__update" role="status">
      <span className="hb__update-mark" aria-hidden="true">
        <ArrowUp size={12} strokeWidth={2} />
      </span>
      <div className="hb__update-text">
        <p className="hb__update-title">Crossup {update.version} is available</p>
        <p className="hb__update-note">
          {working
            ? `Downloading ${progress}…`
            : (update.notes?.split("\n")[0] ??
              "A newer build is ready to replace this one.")}
        </p>
      </div>
      {working ? (
        <span className="hb__update-state">
          {update.status === "downloading" ? progress : "installing…"}
        </span>
      ) : (
        <>
          <button
            type="button"
            className="hb__update-go"
            onClick={() => {
              void updateInstall().catch(() => {});
            }}
          >
            Update now
          </button>
          <button
            type="button"
            className="hb__update-later"
            onClick={() => setLater(update.version)}
          >
            Later
          </button>
        </>
      )}
    </div>
  );
}
