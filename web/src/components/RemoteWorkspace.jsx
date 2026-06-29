import ConfigProvider from "ant-design-vue/es/config-provider";
import RemoteShell from "./RemoteShell.jsx";

/**
 * Renders the remote workspace entry.
 *
 * Applies shared UI tokens and delegates visible business UI to child modules.
 */
export default function RemoteWorkspace() {
  return (
    <ConfigProvider
      theme={{
        token: {
          colorPrimary: "#2563EB",
          borderRadius: 6,
        },
      }}
    >
      <RemoteShell />
    </ConfigProvider>
  );
}
