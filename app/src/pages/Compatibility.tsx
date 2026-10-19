import { mdiCheckDecagram, mdiClipboardCheckOutline, mdiContentCopy, mdiRefresh } from "@mdi/js";
import type { Shared } from "../App";
import { t, type Key } from "../i18n";
import { Button, Card, CheckList, Chip, PageHead, useAction } from "../ui";

const TONE = { compatible: "pass", compatible_after_setup: "missing", incompatible: "fail" } as const;

export function Compatibility(p: Shared) {
  const { busy, run } = useAction(p.setNotice);
  const r = p.report!;

  const copy = async () => {
    await navigator.clipboard.writeText(JSON.stringify(r, null, 2));
    return true;
  };

  return (
    <div className="page">
      <PageHead
        title={t("compat.title")}
        actions={
          <>
            <Button kind="tonal" icon={mdiRefresh} busy={busy === "rescan"} onClick={() => run("rescan", p.refreshReport)}>
              {t("compat.rescan")}
            </Button>
            <Button kind="text" icon={mdiContentCopy} busy={busy === "copy"} onClick={() => run("copy", copy, () => t("compat.copied"))}>
              {t("compat.copy")}
            </Button>
          </>
        }
      />
      <section className="card hero idle">
        <div className="row" style={{ marginTop: 0 }}>
          <Chip tone={TONE[r.verdict]} icon={mdiCheckDecagram}>
            {t(`compat.verdict.${r.verdict}` as Key)}
          </Chip>
        </div>
        <p className="muted" style={{ marginBottom: 0 }}>
          {t("compat.note", { host: r.host.hostname ?? "?", os: r.host.os.pretty_name, kernel: r.host.kernel.release })}
        </p>
      </section>
      {r.groups.map((g) => (
        <Card key={g.title} title={g.title} icon={mdiClipboardCheckOutline}>
          <CheckList checks={g.checks} />
        </Card>
      ))}
    </div>
  );
}
