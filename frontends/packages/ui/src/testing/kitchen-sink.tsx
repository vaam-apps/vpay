import { PAYMENT_STATUS } from "@vpay/tokens";

import { Alert } from "../components/alert";
import { Badge } from "../components/badge";
import { Button } from "../components/button";
import { Card, CardBody } from "../components/card";
import { Checkbox } from "../components/checkbox";
import { Code } from "../components/code";
import { DataList, DataListRow } from "../components/data-list";
import { Dialog } from "../components/dialog";
import { Drawer } from "../components/drawer";
import { EmptyState } from "../components/empty-state";
import { Field, FieldDescription, FieldLabel } from "../components/field";
import { Heading } from "../components/heading";
import { Input } from "../components/input";
import { Link } from "../components/link";
import { List } from "../components/list";
import { LiveRegion } from "../components/live-region";
import { PageShell } from "../components/page-shell";
import { Pagination } from "../components/pagination";
import { Radio, RadioGroup } from "../components/radio";
import { Section } from "../components/section";
import { Select } from "../components/select";
import { Spinner } from "../components/spinner";
import { Stack } from "../components/stack";
import { StatusBadge } from "../components/status-badge";
import { Table } from "../components/table";
import { Text } from "../components/text";
import { Timeline } from "../components/timeline";

/**
 * One render tree covering every `@vpay/ui` export, checked against
 * axe-core's structural rules — plan §7 row 5 ("axe-core@4.13.0 in vitest
 * over every @vpay/ui component ... 0 violations for label, button-name,
 * aria-*, region, list"). Contrast is deliberately not checked here; see
 * `src/testing/axe.ts`.
 */
export function KitchenSink() {
  return (
    <PageShell>
      <Heading level={1}>Confirm your payment</Heading>
      <Text tone="muted">Support line</Text>

      <Stack gap="sm">
        <Button>Continue</Button>
        <Button variant="ghost" size="sm">
          Back
        </Button>
      </Stack>

      <Alert tone="warning">This payment was canceled.</Alert>

      <Card>
        <CardBody>
          <Text>10,000 XAF</Text>
        </CardBody>
      </Card>

      <List>
        <li>MTN Mobile Money is not available for this amount.</li>
      </List>

      <Field>
        <FieldLabel>Phone number</FieldLabel>
        <Input placeholder="+237 6XX XXX XXX" />
        <FieldDescription>Include the country code.</FieldDescription>
      </Field>

      <Checkbox aria-label="Remember this number" />

      <RadioGroup aria-label="Surface" defaultValue="hosted">
        <label>
          <Radio value="hosted" /> Hosted
        </label>
        <label>
          <Radio value="embedded" /> Embedded
        </label>
      </RadioGroup>

      <Select
        items={[
          { value: "en", label: "English" },
          { value: "fr", label: "Français" },
        ]}
        defaultValue="en"
        aria-label="Locale"
      />

      <Table>
        <thead>
          <tr>
            <th>Item</th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td>Coffee</td>
          </tr>
        </tbody>
      </Table>

      <Spinner label="Waiting for the payer" />

      <div>
        {PAYMENT_STATUS.map((status) => (
          <StatusBadge key={status} status={status} />
        ))}
        <Badge tone="ghost">Extra</Badge>
      </div>

      <Section title="Charge">
        <DataList>
          <DataListRow label="Id">
            <Code wrap="anywhere">pi_test_000000000000000001</Code>
          </DataListRow>
          <DataListRow label="Amount">10,000 XAF</DataListRow>
        </DataList>
      </Section>

      <Timeline
        items={[
          { id: "evt_1", at: "2026-09-11 09:00:00", label: "charge.succeeded" },
        ]}
        emptyMessage="No events yet."
      />

      <EmptyState title="No payments" description="None matched." />

      <Pagination
        label="Payments paging"
        previous={null}
        next={<a href="#next">Next</a>}
      />

      <Link href="#back">Back to payments</Link>

      <LiveRegion>
        <Text>Waiting for your approval.</Text>
      </LiveRegion>

      <Dialog.Root open>
        <Dialog.Portal>
          <Dialog.Popup>
            <Dialog.Title>Sign-in failed</Dialog.Title>
            <Dialog.Description>
              Check the code and try again.
            </Dialog.Description>
            <Dialog.Close>Dismiss</Dialog.Close>
          </Dialog.Popup>
        </Dialog.Portal>
      </Dialog.Root>

      <Drawer open onOpenChange={() => {}} title="Charge detail">
        <Text>10,000 XAF · MTN Mobile Money · succeeded</Text>
      </Drawer>
    </PageShell>
  );
}
