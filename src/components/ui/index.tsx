import * as React from 'react';
import {
  Button as HeroButton,
  Card as HeroCard,
  CardBody,
  CardFooter,
  CardHeader,
  Input as HeroInput,
  Modal,
  ModalBody,
  ModalContent,
  ModalFooter,
  ModalHeader,
  Select as HeroSelect,
  SelectItem,
  Switch as HeroSwitch,
  Checkbox as HeroCheckbox,
  Chip,
  Tabs as HeroTabs,
  Tab as HeroTab,
  type ButtonProps as HeroButtonProps,
  type CardProps as HeroCardProps,
  type InputProps as HeroInputProps,
  type ModalProps,
  type SelectProps as HeroSelectProps,
  type SwitchProps as HeroSwitchProps,
  type CheckboxProps as HeroCheckboxProps,
  type ChipProps,
} from '@heroui/react';
import { cn } from '@/lib/utils';

type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';
type ButtonSize = 'sm' | 'md' | 'icon';

export interface ButtonProps extends Omit<HeroButtonProps, 'variant' | 'size' | 'color'> {
  variant?: ButtonVariant;
  size?: ButtonSize;
}

// Section - 用于页面分组，Card 容器 + 标题/描述 + 可选 trailing
export interface SectionProps {
  title: string;
  description: string;
  trailing?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
}

export function Section({ title, description, trailing, children, className }: SectionProps) {
  return (
    <Card className={cn('p-6', className)}>
      <div className="mb-5 flex items-start justify-between gap-4">
        <div>
          <h2 className="text-base font-semibold">{title}</h2>
          <p className="mt-1 text-sm text-default-500">{description}</p>
        </div>
        {trailing}
      </div>
      <div className="space-y-5">{children}</div>
    </Card>
  );
}

// Field - 表单字段行，标签-控件栅格布局
export interface FieldProps {
  label: string;
  hint?: string;
  children: React.ReactNode;
  className?: string;
}

export function Field({ label, hint, children, className }: FieldProps) {
  return (
    <div className={cn('grid grid-cols-1 items-start gap-2 sm:grid-cols-[180px_minmax(0,1fr)] sm:items-center sm:gap-4', className)}>
      <div>
        <Label>{label}</Label>
        {hint && <p className="mt-1 text-xs text-default-400">{hint}</p>}
      </div>
      <div>{children}</div>
    </div>
  );
}

const buttonVariantMap: Record<ButtonVariant, Pick<HeroButtonProps, 'color' | 'variant' | 'className'>> = {
  primary: { color: 'primary', variant: 'solid' },
  secondary: { color: 'default', variant: 'flat' },
  ghost: { color: 'default', variant: 'light' },
  danger: { color: 'danger', variant: 'flat' },
};

const buttonSizeMap: Record<ButtonSize, HeroButtonProps['size']> = {
  sm: 'sm',
  md: 'md',
  icon: 'sm',
};

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant = 'primary', size = 'md', isIconOnly, children, ...props }, ref) => {
    const mapped = buttonVariantMap[variant];
    const iconOnly = isIconOnly ?? size === 'icon';
    return (
      <HeroButton
        ref={ref}
        radius={size === 'icon' ? 'md' : 'lg'}
        size={buttonSizeMap[size]}
        color={mapped.color}
        variant={mapped.variant}
        isIconOnly={iconOnly}
        className={cn(
          'font-medium transition-transform active:scale-[0.98]',
          variant === 'ghost' && 'text-default-500 data-[hover=true]:text-foreground',
          variant === 'secondary' && 'bg-default-100 text-default-700',
          variant === 'danger' && 'bg-danger-50 text-danger',
          className,
        )}
        {...props}
      >
        {children}
      </HeroButton>
    );
  },
);
Button.displayName = 'Button';

export interface CardProps extends HeroCardProps {
  interactive?: boolean;
}

export const Card = React.forwardRef<HTMLDivElement, CardProps>(
  ({ className, interactive = false, radius = 'lg', shadow = 'none', ...props }, ref) => (
    <HeroCard
      ref={ref}
      radius={radius}
      shadow={shadow}
      isBlurred={false}
      className={cn(
        'border border-divider bg-background',
        interactive && 'transition-colors data-[hover=true]:border-primary/50 data-[hover=true]:bg-default-50',
        className,
      )}
      {...props}
    />
  ),
);
Card.displayName = 'Card';

export { CardBody, CardFooter, CardHeader };

export function Input({ className, variant = 'bordered', radius = 'md', labelPlacement = 'outside', ...props }: HeroInputProps) {
  return (
    <HeroInput
      variant={variant}
      radius={radius}
      labelPlacement={labelPlacement}
      classNames={{
        input: 'text-sm text-foreground',
        inputWrapper: cn('h-9 min-h-9 bg-background', className),
        innerWrapper: 'text-foreground',
        label: 'text-sm font-medium text-foreground',
      }}
      {...props}
    />
  );
}

export interface SelectProps extends Omit<HeroSelectProps, 'children'> {
  children: NonNullable<HeroSelectProps['children']>;
}

export function Select({ className, variant = 'bordered', radius = 'md', children, ...props }: SelectProps) {
  return (
    <HeroSelect
      variant={variant}
      radius={radius}
      classNames={{
        trigger: cn('h-9 min-h-9 bg-background', className),
        value: 'text-sm',
      }}
      {...props}
    >
      {children}
    </HeroSelect>
  );
}

export { SelectItem };
export const SelectOption = SelectItem;
export { Modal, ModalBody, ModalContent, ModalFooter, ModalHeader };
export type { ModalProps };

export function Label({ className, ...props }: React.LabelHTMLAttributes<HTMLLabelElement>) {
  return <label className={cn('text-sm font-medium text-foreground', className)} {...props} />;
}

/**
 * 统一表单复选框。对 HeroUI Checkbox 的语义封装，保证跨页面一致的勾选交互与主题色。
 * 交互 props 遵循 React Aria：isSelected / onValueChange / isDisabled。
 */
export interface CheckboxProps extends Omit<HeroCheckboxProps, 'isSelected' | 'onValueChange' | 'children'> {
  checked?: boolean;
  onCheckedChange?: (checked: boolean) => void;
  label?: React.ReactNode;
  labelClassName?: string;
}

export function Checkbox({ checked, onCheckedChange, label, labelClassName, className, children, ...props }: CheckboxProps & { children?: React.ReactNode }) {
  const content = children ?? label;
  return (
    <label className={cn('flex cursor-pointer items-start gap-2 text-sm', props.isDisabled && 'cursor-not-allowed opacity-50', className)}>
      <HeroCheckbox
        isSelected={checked}
        onValueChange={onCheckedChange}
        aria-label={props['aria-label'] ?? (typeof content === 'string' ? content : undefined)}
        className="mt-0.5 shrink-0"
        {...props}
      />
      {content != null && <span className={cn('text-default-600 leading-normal', labelClassName)}>{content}</span>}
    </label>
  );
}

/**
 * 紧凑分段选择器：适合 2–4 个互斥选项（如「串行/并行」），
 * 视觉上是带边框的分段按钮组，比下拉 Select 更直观。
 */
export interface SelectButtonGroupOption<T extends string> {
  value: T;
  label: React.ReactNode;
  title?: string;
}

export interface SelectButtonGroupProps<T extends string> {
  value: T;
  onValueChange: (value: T) => void;
  options: SelectButtonGroupOption<T>[];
  isDisabled?: boolean;
  size?: 'sm' | 'md';
  'aria-label'?: string;
  className?: string;
}

export function SelectButtonGroup<T extends string>({
  value,
  onValueChange,
  options,
  isDisabled,
  size = 'sm',
  'aria-label': ariaLabel,
  className,
}: SelectButtonGroupProps<T>) {
  return (
    <div
      role="group"
      aria-label={ariaLabel}
      className={cn('inline-flex items-center gap-0.5 rounded-lg border border-divider bg-default-100 p-0.5', className)}
    >
      {options.map((option) => {
        const active = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            title={option.title}
            aria-pressed={active}
            disabled={isDisabled}
            onClick={() => onValueChange(option.value)}
            className={cn(
              'rounded-md font-medium transition-colors',
              size === 'sm' ? 'px-2.5 py-1 text-xs' : 'px-3 py-1.5 text-sm',
              active
                ? 'bg-background text-foreground shadow-sm'
                : 'text-default-500 hover:text-foreground disabled:hover:text-default-500',
              isDisabled && 'cursor-not-allowed opacity-50',
            )}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

/**
 * 设置区段：Card 容器 + 标题/描述 + 垂直表单栈。
 * SettingsPage 与各配置页统一使用，保证页面风格一致。
 */
export function FormSection({
  title,
  description,
  children,
  className,
}: {
  title: string;
  description?: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <Card className={cn('rounded-2xl border-divider bg-default-50/50 p-6', className)}>
      <div className="mb-5">
        <h2 className="text-base font-semibold">{title}</h2>
        {description && <p className="mt-1 text-sm text-default-500">{description}</p>}
      </div>
      <div className="space-y-5">{children}</div>
    </Card>
  );
}

/**
 * 表单字段行：左栏 label + hint，右栏控件 + error。
 * 与 SettingsPage 的网格布局保持一致（sm 以上 180px 标签列）。
 * labelWidth 覆盖标签列宽度（'auto' 用于内联工具栏等紧凑场景）。
 */
export function FormField({
  label,
  hint,
  fieldId,
  error,
  children,
  className,
  labelAuto = false,
}: {
  label: React.ReactNode;
  hint?: string;
  fieldId?: string;
  error?: string;
  children: React.ReactNode;
  className?: string;
  /** true = 标签列用 auto 宽度（适合内联工具栏）；false = 固定 180px（默认，表单页风格）。 */
  labelAuto?: boolean;
}) {
  return (
    <div
      className={cn(
        'grid grid-cols-1 items-start gap-2 sm:gap-4',
        labelAuto ? 'sm:grid-cols-[auto_minmax(0,1fr)]' : 'sm:grid-cols-[180px_minmax(0,1fr)]',
        className
      )}
    >
      <div>
        <Label htmlFor={fieldId}>{label}</Label>
        {hint && <p className="mt-1 text-xs text-default-400">{hint}</p>}
      </div>
      <div>
        {children}
        {error && (
          <p id={fieldId ? `${fieldId}-error` : undefined} role="alert" className="mt-1.5 text-sm text-danger">
            {error}
          </p>
        )}
      </div>
    </div>
  );
}

export interface SwitchProps extends Omit<HeroSwitchProps, 'onChange'> {
  checked?: boolean;
  onCheckedChange?: (checked: boolean) => void;
  label?: string;
}

export function Switch({ checked, onCheckedChange, label, className, size = 'sm', color = 'primary', ...props }: SwitchProps) {
  return (
    <HeroSwitch
      size={size}
      color={color}
      isSelected={checked}
      onValueChange={onCheckedChange}
      aria-label={label}
      className={className}
      {...props}
    />
  );
}

export function Badge({ className, children, ...props }: ChipProps) {
  return (
    <Chip
      size="sm"
      radius="sm"
      variant="flat"
      className={cn('bg-default-100 text-default-700', className)}
      {...props}
    >
      {children}
    </Chip>
  );
}

export interface ProgressProps {
  value: number | null;
  label?: string;
  className?: string;
}

export function Progress({ value, label, className }: ProgressProps) {
  const percent = value == null ? null : Math.max(0, Math.min(100, value));
  return (
    <div className={cn('space-y-1', className)}>
      <div
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
        className="h-2 w-full overflow-hidden rounded-full bg-default-100"
      >
        <div
          className={cn(
            'h-full rounded-full bg-primary transition-[width] duration-200',
            percent == null && 'w-1/3 animate-pulse',
          )}
          style={percent == null ? undefined : { width: `${percent}%` }}
        />
      </div>
    </div>
  );
}

export interface TabsProps {
  value: string;
  onValueChange: (key: string) => void;
  children: React.ReactNode;
  className?: string;
  'aria-label'?: string;
}

/**
 * 简单受控 Tabs：以 <Tab tabKey= title=> 子元素驱动。
 */
export function Tabs({ value, onValueChange, children, className, 'aria-label': ariaLabel }: TabsProps) {
  const tabs: { key: string; title: React.ReactNode; content: React.ReactNode }[] = [];
  React.Children.forEach(children, (child) => {
    if (!React.isValidElement(child)) return;
    const props = (child as React.ReactElement<TabProps>).props;
    const key = props.tabKey;
    if (key == null) return;
    tabs.push({ key: String(key), title: props.title ?? key, content: props.children });
  });
  return (
    <HeroTabs
      aria-label={ariaLabel}
      selectedKey={value}
      onSelectionChange={(k: React.Key) => onValueChange(String(k))}
      className={className}
      variant="underlined"
    >
      {tabs.map((t) => (
        <HeroTab key={t.key} title={t.title}>
          <div className="pt-4">{t.content}</div>
        </HeroTab>
      ))}
    </HeroTabs>
  );
}

export interface TabProps {
  tabKey: string;
  title: React.ReactNode;
  children: React.ReactNode;
}

export function Tab(_: TabProps) {
  return null;
}
