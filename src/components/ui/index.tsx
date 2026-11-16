import * as React from 'react';
import {
  Button as HeroButton,
  Card as HeroCard,
  CardBody,
  CardFooter,
  CardHeader,
  Input as HeroInput,
  Select as HeroSelect,
  SelectItem,
  Switch as HeroSwitch,
  Chip,
  type ButtonProps as HeroButtonProps,
  type CardProps as HeroCardProps,
  type InputProps as HeroInputProps,
  type SelectProps as HeroSelectProps,
  type SwitchProps as HeroSwitchProps,
  type ChipProps,
} from '@heroui/react';
import { cn } from '@/lib/utils';

type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';
type ButtonSize = 'sm' | 'md' | 'icon';

export interface ButtonProps extends Omit<HeroButtonProps, 'variant' | 'size' | 'color'> {
  variant?: ButtonVariant;
  size?: ButtonSize;
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

export function Input({ className, variant = 'bordered', radius = 'md', ...props }: HeroInputProps) {
  return (
    <HeroInput
      variant={variant}
      radius={radius}
      classNames={{
        input: 'text-sm',
        inputWrapper: cn('h-9 min-h-9 bg-background', className),
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

export function Label({ className, ...props }: React.LabelHTMLAttributes<HTMLLabelElement>) {
  return <label className={cn('text-sm font-medium text-foreground', className)} {...props} />;
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
