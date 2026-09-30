// Placeholder page for sections implemented in later phases (Section 30 navigation shell).
export default function Placeholder({ title, phase }: { title: string; phase: string }) {
  return (
    <div className="page">
      <h1>{title}</h1>
      <div className="card muted">
        <p>
          This section arrives in <strong>{phase}</strong>.
        </p>
        <p className="small">
          Phase 1 delivers the foundation and a working, verified backup engine. Use{" "}
          <strong>Backup Now</strong> to run a one-off local backup end-to-end.
        </p>
      </div>
    </div>
  );
}
