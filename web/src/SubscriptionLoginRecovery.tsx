import { Alert, Button, Stack, Typography } from "@mui/material";
import type { subscriptionLogin, LoginView } from "./subscriptionLoginController";

export function SubscriptionLoginRecovery({ owner, view, reference, onClose }: {
  owner: ReturnType<typeof subscriptionLogin>; view: LoginView; reference?: string | undefined; onClose: () => void;
}): React.JSX.Element {
  return <Stack spacing={1.5}>
    <Alert severity="warning">The saved sign-in could not be read. Authorization stays blocked because a code may already have been submitted.</Alert>
    <Typography variant="body2">Retry reading the saved record. To inspect an existing sign-in, open its Details or Continue sign-in entry in Subscriptions. Checking a receipt does not restart authorization or clear the saved record.</Typography>
    {view.error && <Alert severity="warning">{view.error}</Alert>}
    {view.observation && <Typography role="status">Server sign-in status: {view.observation.status}. The local record still needs recovery.</Typography>}
    <Button disabled={view.busy} onClick={() => {
      owner.restore();
      if (!owner.snapshot.storageBlocked) {
        if (reference) void owner.resume(reference);
        else if (owner.snapshot.pending) void owner.check();
      }
    }}>Retry saved record</Button>
    {reference && <Button disabled={view.busy} onClick={() => void owner.inspect(reference)}>Check server sign-in</Button>}
    <Button onClick={onClose}>Back to Subscriptions</Button>
  </Stack>;
}
