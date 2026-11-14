export type OperatorBrowserConfig = Readonly<{
  enabled: boolean;
  prefix: '/__operator/api';
  attachmentIdentity: string | null;
}>;

declare const __SCREENROOM_WEB_OPERATOR__: OperatorBrowserConfig;

export const operatorConfig: OperatorBrowserConfig = Object.freeze(__SCREENROOM_WEB_OPERATOR__);
