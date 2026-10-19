import type { Shared } from "../App";
import { Button, Card, CheckList, Chip, useAction } from "../ui";

const VERDICT = {
  compatible: ["pass", "Compatibile"],
  compatible_after_setup: ["missing", "Compatibile dopo l'installazione dei componenti mancanti"],
  incompatible: ["fail", "Non compatibile"],
} as const;

export function Compatibility(p: Shared) {
  const { busy, run } = useAction(p.setNotice);
  const r = p.report!;
  const [tone, label] = VERDICT[r.verdict];

  const copy = async () => {
    await navigator.clipboard.writeText(JSON.stringify(r, null, 2));
    return true;
  };

  return (
    <div className="page">
      <h1>Report di compatibilità</h1>
      <Card
        title={
          <>
            Esito: <Chip tone={tone}>{label}</Chip>
          </>
        }
        actions={
          <>
            <Button busy={busy === "rescan"} onClick={() => run("rescan", p.refreshReport)}>
              Rianalizza
            </Button>
            <Button busy={busy === "copy"} onClick={() => run("copy", copy, () => "Report JSON copiato negli appunti")}>
              Copia JSON
            </Button>
          </>
        }
      >
        <p className="muted small">
          {r.host.hostname} · {r.host.os.pretty_name} · kernel {r.host.kernel.release}. Il supporto è valutato sulle caratteristiche reali del sistema, non sul nome della
          distribuzione.
        </p>
      </Card>
      {r.groups.map((g) => (
        <Card key={g.title} title={g.title}>
          <CheckList checks={g.checks} />
        </Card>
      ))}
    </div>
  );
}
