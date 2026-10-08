// Every control the app uses, on one page, so they can be compared against real AppKit controls
// (scripts/appkit-reference.swift). Only reachable from the dev server.

import { useState } from "react";
import { Toggle } from "../components/Form";
import { Icon } from "../components/Icon";
import { PopUpButton } from "../components/PopUpButton";
import "../settings/Settings.css";
import "./Gallery.css";

export function Gallery() {
  const [account, setAccount] = useState("work");
  const [kind, setKind] = useState<"github" | "gitlab">("github");
  const [on, setOn] = useState(true);
  return (
    <main className="gallery">
      <div className="gallery__row">
        <span>push</span>
        <button type="button">Replace Token…</button>
      </div>
      <div className="gallery__row">
        <span>default</span>
        <button type="button" className="default">
          Add Account
        </button>
      </div>
      <div className="gallery__row">
        <span>popup</span>
        <PopUpButton
          label="Account"
          value={account}
          onChange={setAccount}
          options={[
            { value: "work", label: "Work" },
            { value: "personal", label: "Personal" },
            { value: "corp", label: "Corp GitLab" },
          ]}
        />
      </div>
      <div className="gallery__row">
        <span>field</span>
        <input className="field" placeholder="github_pat_…" />
      </div>
      <div className="gallery__row">
        <span>search</span>
        <label className="search">
          <Icon name="search" size={15} />
          <input type="search" placeholder="Search" />
        </label>
      </div>
      <div className="gallery__row">
        <span>segmented</span>
        <div className="segmented" role="radiogroup">
          {(["github", "gitlab"] as const).map((k) => (
            <button
              key={k}
              type="button"
              role="radio"
              aria-checked={kind === k}
              className={kind === k ? "segmented__item segmented__item--on" : "segmented__item"}
              onClick={() => setKind(k)}
            >
              {k === "github" ? "GitHub" : "GitLab"}
            </button>
          ))}
        </div>
      </div>
      <div className="gallery__row">
        <span>switch</span>
        <Toggle label="switch" checked={on} onChange={setOn} />
      </div>
      <div className="gallery__row">
        <span>check-on</span>
        <label className="check">
          <input type="checkbox" defaultChecked /> billing-api
        </label>
      </div>
      <div className="gallery__row">
        <span>check-off</span>
        <label className="check">
          <input type="checkbox" /> sandbox
        </label>
      </div>
      <div className="gallery__row">
        <span>check-mixed</span>
        <label className="check">
          <input
            type="checkbox"
            ref={(el) => {
              if (el) {
                el.indeterminate = true;
              }
            }}
          />{" "}
          acme
        </label>
      </div>
      <div className="gallery__row">
        <span>plus-minus</span>
        <div className="add-remove">
          <button type="button" aria-label="Add">
            <Icon name="plus" size={13} />
          </button>
          <button type="button" aria-label="Remove">
            <Icon name="minus" size={13} />
          </button>
        </div>
      </div>
    </main>
  );
}
