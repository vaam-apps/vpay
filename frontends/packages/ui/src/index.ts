/**
 * `@vpay/ui` — the one primitive set both the checkout and the dashboard
 * compose from.
 *
 * Every entry below is a folder under `src/components/`, holding the
 * component, its `cva` variant map, its test and its story. Nothing in an
 * app writes a class name; see `just verify-ui` for the gates that say so,
 * including the 200-line ceiling on every file in this package.
 */
export { cn } from "./cn";

export { Alert, alert, type AlertProps } from "./components/alert";
export { Badge, badge, type BadgeProps } from "./components/badge";
export { Button, button, type ButtonProps } from "./components/button";
export {
  Card,
  card,
  CardBody,
  type CardBodyProps,
  type CardProps,
} from "./components/card";
export {
  Checkbox,
  checkbox,
  CheckboxLabel,
  type CheckboxLabelProps,
  type CheckboxProps,
} from "./components/checkbox";
export { Code, type CodeProps } from "./components/code";
export {
  DataList,
  type DataListProps,
  DataListRow,
  type DataListRowProps,
} from "./components/data-list";
export {
  Dialog,
  DialogClose,
  DialogDescription,
  type DialogDescriptionProps,
  DialogPopup,
  type DialogPopupProps,
  DialogPortal,
  type DialogPortalProps,
  DialogRoot,
  DialogTitle,
  type DialogTitleProps,
  DialogTrigger,
} from "./components/dialog";
export { Drawer, type DrawerProps } from "./components/drawer";
export { EmptyState, type EmptyStateProps } from "./components/empty-state";
export {
  Field,
  FieldDescription,
  type FieldDescriptionProps,
  FieldError,
  type FieldErrorProps,
  FieldLabel,
  type FieldLabelProps,
  type FieldProps,
} from "./components/field";
export { Heading, type HeadingProps } from "./components/heading";
export { Input, input, type InputProps } from "./components/input";
export { Link, link, type LinkProps } from "./components/link";
export { List, type ListProps } from "./components/list";
export { LiveRegion, type LiveRegionProps } from "./components/live-region";
export { Logo, type LogoProps } from "./components/logo";
export { PageShell, type PageShellProps } from "./components/page-shell";
export { Pagination, type PaginationProps } from "./components/pagination";
export {
  Radio,
  radio,
  RadioGroup,
  type RadioGroupProps,
  type RadioProps,
} from "./components/radio";
export { Section, type SectionProps } from "./components/section";
export {
  Select,
  type SelectItem,
  type SelectProps,
  selectTrigger,
} from "./components/select";
export { Spinner, spinner, type SpinnerProps } from "./components/spinner";
export { Stack, stack, type StackProps } from "./components/stack";
export { StatusBadge, type StatusBadgeProps } from "./components/status-badge";
export { Table, table, type TableProps } from "./components/table";
export { Text, text, type TextProps } from "./components/text";
export {
  Timeline,
  type TimelineItem,
  type TimelineProps,
} from "./components/timeline";
export {
  VisuallyHidden,
  type VisuallyHiddenProps,
} from "./components/visually-hidden";
