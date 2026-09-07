import { render } from '@testing-library/react';
import { PAYMENT_STATUS } from '@vpay/tokens';
import { describe, expect, it } from 'vitest';

import { Alert } from './components/alert.js';
import { Badge } from './components/badge.js';
import { Button } from './components/button.js';
import { Card, CardBody } from './components/card.js';
import { Checkbox } from './components/checkbox.js';
import { Dialog } from './components/dialog.js';
import { Drawer } from './components/drawer.js';
import { Field, FieldDescription, FieldLabel } from './components/field.js';
import { Heading, List, PageShell, Stack, Text } from './components/layout.js';
import { Input } from './components/input.js';
import { Radio, RadioGroup } from './components/radio.js';
import { Select } from './components/select.js';
import { Spinner } from './components/spinner.js';
import { StatusBadge } from './components/status-badge.js';
import { Table } from './components/table.js';
import { axeViolations } from './testing/axe.js';

/**
 * One render tree covering every `@vpay/ui` export, checked against
 * axe-core's structural rules — plan §7 row 5 ("axe-core@4.13.0 in vitest
 * over every @vpay/ui component ... 0 violations for label, button-name,
 * aria-*, region, list"). Contrast is deliberately not checked here; see
 * `src/testing/axe.ts`.
 */
function KitchenSink() {
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
          { value: 'en', label: 'English' },
          { value: 'fr', label: 'Français' },
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

      <Dialog.Root open>
        <Dialog.Portal>
          <Dialog.Popup>
            <Dialog.Title>Sign-in failed</Dialog.Title>
            <Dialog.Description>Check the code and try again.</Dialog.Description>
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

describe('@vpay/ui structural accessibility', () => {
  it('has zero violations for label, button-name, aria-*, region and list', async () => {
    const { unmount } = render(<KitchenSink />);
    const violations = await axeViolations(document.body);
    expect(violations).toEqual([]);
    unmount();
  });
});
