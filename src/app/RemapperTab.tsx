import { PressDepth } from "@/components/interior/press-depth";
import type { MappingView, View } from "@/bridge";
import { Switch } from "./Dialogs";
import { MappingTable } from "./Mappings";
import { Sidebar } from "./Sidebar";

/**
 * The original tool, unchanged: profiles on the left, the mapping table for the
 * selected profile on the right.
 *
 * Two switches sit above that table and they mean different things, so they are
 * spelled out rather than left as two identical toggles a row apart: one turns
 * the whole remapper off, the other switches off just the mappings of the
 * profile on screen.
 */
export function RemapperTab({
  view,
  selected,
  onSelect,
  onAddProfile,
  onEdit,
  run,
}: {
  view: View;
  selected: string;
  onSelect: (id: string) => void;
  onAddProfile: () => void;
  onEdit: (mapping: MappingView | null) => void;
  run: (command: string, args?: Record<string, unknown>) => void;
}) {
  const profile =
    view.profiles.find((entry) => entry.id === selected) ?? view.profiles[0];

  return (
    <div className="hb__split">
      <Sidebar
        profiles={view.profiles}
        selected={profile.id}
        activeProgram={view.program}
        hookEnabled={view.hook_enabled}
        onSelect={onSelect}
        onAdd={onAddProfile}
      />

      <section className="hb__page">
        <div className="hb__head">
          <div style={{ minWidth: 0 }}>
            <h1 className="hb__title">
              {profile.global ? "Crossup Global" : profile.program}
            </h1>
            <p className="hb__sub">
              {profile.global
                ? "applies to every program without its own profile"
                : `${profile.mappings.length} mapping${profile.mappings.length === 1 ? "" : "s"} · ${
                    profile.active ? "live while this program is in front" : "off for this program"
                  }`}
            </p>
          </div>
          <span className="hb__spacer" />
        </div>

        <div className="hb__switch-rows">
          <div className="hb__row">
            <span className="hb__row-text">
              <span className="hb__row-name">Remapper</span>
              <span className="hb__row-note">
                Off lets every key through untouched, in every program.
              </span>
            </span>
            <Switch
              checked={view.hook_enabled}
              label="Remapper enabled"
              onChange={(enabled) =>
                run("set_tool_enabled", { tool: "remapper", enabled })
              }
            />
          </div>

          {!profile.global && (
            <div className="hb__row">
              <span className="hb__row-text">
                <span className="hb__row-name">
                  These mappings, in {profile.program}
                </span>
                <span className="hb__row-note">
                  Off skips them while that program is in front. The remapper stays
                  on everywhere else.
                </span>
              </span>
              <Switch
                checked={profile.active}
                label={`Mappings on in ${profile.program}`}
                onChange={(active) =>
                  run("set_profile_active", { id: profile.id, active })
                }
              />
            </div>
          )}
        </div>

        <div className="hb__actions">
          <PressDepth depth={3} tilt={5} onClick={() => onEdit(null)}>
            Add mapping
          </PressDepth>
          <select
            className="hb__field"
            style={{ width: "auto", cursor: "pointer" }}
            aria-label="Apply a preset"
            value=""
            onChange={(event) => {
              if (!event.target.value) return;
              void run("apply_preset", {
                profile: profile.id,
                preset: event.target.value,
              });
            }}
          >
            <option value="">Apply preset…</option>
            <option value="wasd">WASD</option>
            <option value="arrows">Arrow keys</option>
            <option value="both">Both</option>
          </select>
        </div>

        {!view.hook_enabled && (
          <p className="hb__warn">
            The remapper is switched off: every key reaches the program in front
            untouched, whatever the mappings below say.
          </p>
        )}
        {!profile.global && !profile.active && (
          <p className="hb__warn">
            This profile is switched off, so its mappings are skipped while{" "}
            {profile.program} is in front — the one it would otherwise apply to.
          </p>
        )}

        <MappingTable
          mappings={profile.mappings}
          onAdd={() => onEdit(null)}
          onDelete={(index) => run("remove_mapping", { profile: profile.id, index })}
          onPreset={(preset) => run("apply_preset", { profile: profile.id, preset })}
        />
      </section>
    </div>
  );
}
