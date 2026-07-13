import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Widgets

Item {
    id: root
    property var pluginApi: null
    readonly property var geometryPlaceholder: panelContainer
    readonly property bool allowAttach: true
    property real contentPreferredWidth: Math.round(420 * Style.uiScaleRatio)
    property real contentPreferredHeight: Math.round(480 * Style.uiScaleRatio)
    property color panelBackgroundColor: Qt.alpha(Color.mSurface, 0.78)
    readonly property color glassCardColor: Qt.alpha(Color.mSurfaceVariant, 0.56)

    property string activePage: "overview"
    property bool offline: false
    property string accountName: "LLM Meter"
    property string connectionStatus: "正在读取"
    property string quotaName: "当前额度"
    property int quotaPercent: -1
    property string quotaResetsAt: "—"
    property string quotaForecast: "正在积累本地用量样本"
    property string todayTokens: "—"
    property string todayCost: "—"
    property string modelSummary: "Provider 未返回模型维度"
    property var trendValues: [0, 0, 0, 0, 0, 0, 0]
    property real trendMaximum: 1
    property string trendTotal: "—"
    property var connections: []
    property string selectedConnectionId: ""
    readonly property var selectedConnection: root.connections.find(connection => connection.id === root.selectedConnectionId) || (root.connections.length ? root.connections[0] : null)
    property var localCodex: ({
            active_sessions: [],
            models: []
        })
    property string actionMessage: ""
    property string pendingRemoveId: ""
    property bool loginCancelled: false
    property bool autostartEnabled: false
    property bool autostartActive: false
    property bool autostartKnown: false

    property var loginOptions: []
    property bool addAccountOpen: false
    property var selectedLoginOption: null
    property bool apiKeyMode: false
    property string apiKeyInput: ""
    property string pendingProcessSecret: ""
    property bool relayMode: false
    property string relayDisplayName: "My Provider"
    property string relayBaseUrl: ""
    property string relayListen: "127.0.0.1:18456"
    property bool proxyOpen: false
    property var selectedProxy: null
    property bool proxyRunning: false
    property string proxyListen: "127.0.0.1:18456"
    property var proxyCredentials: []
    property string proxyCredentialName: "Client"
    property string proxyCredentialToken: ""
    property string createdProxyToken: ""

    function openAddAccount() {
        root.addAccountOpen = true;
        root.apiKeyMode = false;
        root.selectedLoginOption = null;
        root.apiKeyInput = "";
        root.relayMode = false;
        root.relayDisplayName = "My Provider";
        root.relayBaseUrl = "";
        root.refreshLoginProviders();
    }
    function closeAddAccount() {
        root.addAccountOpen = false;
        root.apiKeyMode = false;
        root.selectedLoginOption = null;
        root.apiKeyInput = "";
        root.relayMode = false;
        root.relayBaseUrl = "";
    }
    function parseLoginOptions(text) {
        try {
            const data = JSON.parse(String(text));
            const providers = data.providers || [];
            const options = [];
            providers.forEach(provider => {
                (provider.connection_types || []).forEach(type => {
                    const schemes = type.auth_schemes || [];
                    const is_oauth = schemes.some(s => s === "oauth_browser" || s === "oauth_device_code");
                    const is_api_key = schemes.some(s => s === "admin_api_key" || s === "api_key");
                    if (!is_oauth && !is_api_key)
                        return;
                    const label = type.display_name || provider.display_name || provider.provider_id;
                    const auth_scheme = is_oauth ? (schemes.find(s => s === "oauth_browser" || s === "oauth_device_code") || schemes[0]) : (schemes.find(s => s === "admin_api_key" || s === "api_key") || schemes[0]);
                    options.push({
                        provider_id: provider.provider_id,
                        provider_display_name: provider.display_name,
                        connection_type: type.id,
                        connection_type_display_name: type.display_name,
                        label: label,
                        auth_scheme: auth_scheme,
                        is_oauth: is_oauth,
                        is_api_key: is_api_key
                    });
                });
            });
            root.loginOptions = options;
        } catch (error) {
            root.loginOptions = [];
        }
    }
    function selectLoginOption(option) {
        if (option.provider_id === "relay" && option.connection_type === "openai_compatible_proxy") {
            root.selectedLoginOption = option;
            root.apiKeyMode = true;
            root.relayMode = true;
            root.apiKeyInput = "";
        } else if (option.is_oauth) {
            root.startOAuthLogin(option);
            root.closeAddAccount();
        } else if (option.is_api_key) {
            root.selectedLoginOption = option;
            root.apiKeyMode = true;
            root.relayMode = false;
            root.apiKeyInput = "";
        }
    }
    function startOAuthLogin(option) {
        if (root.busy)
            return;
        root.loginCancelled = false;
        loginProcess.command = [root.cli, "add", "subscription", "--provider", option.provider_id, "--name", option.label, "--open"];
        root.actionMessage = "已打开浏览器，正在等待登录完成…";
        loginProcess.running = true;
    }
    function submitApiKey() {
        if (root.busy || !root.selectedLoginOption)
            return;
        const option = root.selectedLoginOption;
        root.pendingProcessSecret = root.apiKeyInput;
        if (root.relayMode) {
            actionProcess.command = [root.cli, "add", "relay", "--profile", "generic", "--base-url", root.relayBaseUrl.trim(), "--listen", root.relayListen.trim(), "--name", root.relayDisplayName.trim(), "--secret-stdin"];
            root.actionMessage = "正在验证模型接口并创建本地代理…";
        } else {
            const kind = option.auth_scheme === "admin_api_key" ? "admin" : "standard";
            actionProcess.command = [root.cli, "add", kind, "--name", option.label, "--secret-stdin"];
            root.actionMessage = "正在保存 API Key…";
        }
        root.closeAddAccount();
        actionProcess.running = true;
    }
    function refreshLoginProviders() {
        if (!providersProcess.running)
            providersProcess.running = true;
    }
    function openProxy(connection) {
        root.selectedProxy = connection;
        root.proxyOpen = true;
        root.proxyCredentialName = "Client";
        root.proxyCredentialToken = "";
        root.createdProxyToken = "";
        root.refreshProxy();
    }
    function closeProxy() {
        root.proxyOpen = false;
        root.selectedProxy = null;
        root.proxyCredentials = [];
        root.createdProxyToken = "";
    }
    function refreshProxy() {
        if (!root.selectedProxy)
            return;
        proxyStatusProcess.command = [root.cli, "proxy", "status", root.selectedProxy.id];
        proxyCredentialsProcess.command = [root.cli, "proxy", "credentials", root.selectedProxy.id];
        proxyStatusProcess.running = true;
        proxyCredentialsProcess.running = true;
    }
    function toggleProxy() {
        if (!root.selectedProxy || root.busy)
            return;
        root.runAction(["proxy", root.proxyRunning ? "stop" : "start", root.selectedProxy.id], root.proxyRunning ? "正在停止本地代理…" : "正在启动本地代理…");
    }
    function createProxyCredential() {
        if (!root.selectedProxy || root.proxyCredentialName.trim() === "" || credentialCreateProcess.running)
            return;
        const command = [root.cli, "proxy", "create-credential", root.selectedProxy.id, "--name", root.proxyCredentialName.trim()];
        if (root.proxyCredentialToken.trim() !== "")
            command.push("--token", root.proxyCredentialToken.trim());
        credentialCreateProcess.command = command;
        root.createdProxyToken = "";
        credentialCreateProcess.running = true;
    }
    function disableProxyCredential(credentialId) {
        if (!root.selectedProxy || root.busy)
            return;
        root.proxyCredentials = root.proxyCredentials.filter(credential => credential.id !== credentialId);
        root.runAction(["proxy", "disable-credential", root.selectedProxy.id, credentialId], "正在撤销客户端凭证…");
    }
    function copyProxyToken(token) {
        if (!token)
            return;
        clipboardProcess.command = ["wl-copy", "--", token];
        clipboardProcess.running = true;
    }

    readonly property string cli: Quickshell.env("HOME") + "/.local/bin/llm-meter"
    readonly property bool busy: actionProcess.running || loginProcess.running || credentialCreateProcess.running
    readonly property var resetSummaries: root.connections.map(connection => connection.rate_limit_reset_credits).filter(summary => summary !== null && summary !== undefined)
    readonly property int resetAvailableCount: root.resetSummaries.reduce((total, summary) => total + Number(summary.available_count || 0), 0)
    readonly property var resetCreditItems: root.resetSummaries.reduce((items, summary) => items.concat(summary.credits || []), [])
    readonly property bool resetDetailsKnown: root.resetSummaries.some(summary => summary.credits !== null && summary.credits !== undefined)
    readonly property bool hasSubscription: root.connections.some(connection => connection.connection_type === "chatgpt_subscription")
    readonly property var nearestResetCredit: {
        const now = Date.now();
        const available = root.resetCreditItems.filter(credit => credit.status === "available" && credit.expires_at && new Date(credit.expires_at).getTime() > now);
        available.sort((left, right) => new Date(left.expires_at).getTime() - new Date(right.expires_at).getTime());
        return available.length ? available[0] : null;
    }
    readonly property bool nearestResetUrgent: root.nearestResetCredit && new Date(root.nearestResetCredit.expires_at).getTime() - Date.now() < 24 * 60 * 60 * 1000

    function statusText(status) {
        const labels = {
            "ready": "已同步",
            "syncing": "同步中",
            "connecting": "连接中",
            "stale": "数据陈旧",
            "offline": "离线",
            "auth_required": "需要认证",
            "rate_limited": "已限流",
            "provider_error": "Provider 错误",
            "disabled": "已停用"
        };
        return labels[status] || status || "未知";
    }
    function statusColor(status) {
        if (status === "ready")
            return Color.mPrimary;
        if (["auth_required", "offline", "provider_error", "stale", "rate_limited", "disabled"].indexOf(status) >= 0)
            return Color.mError;
        return Color.mOnSurfaceVariant;
    }
    function providerIcon(providerId) {
        const icons = {
            "openai": "terminal-2",
            "kimi": "message-circle"
        };
        return icons[providerId] || "server";
    }
    function connectionQuota(connection) {
        const quotas = (connection && connection.quota_windows) || [];
        let best = null;
        quotas.forEach(quota => {
            if (quota.remaining_ratio === null || quota.remaining_ratio === undefined)
                return;
            if (!best || Number(quota.remaining_ratio) < Number(best.remaining_ratio))
                best = quota;
        });
        return best;
    }
    function connectionQuotaPercent(connection) {
        const quota = root.connectionQuota(connection);
        return quota ? Math.round(Number(quota.remaining_ratio) * 100) : -1;
    }
    function connectionQuotaName(connection) {
        const quota = root.connectionQuota(connection);
        return quota ? (quota.display_name || "当前额度") : "额度未提供";
    }
    function connectionQuotaResetsAt(connection) {
        const quota = root.connectionQuota(connection);
        return quota && quota.resets_at ? root.localDateTime(quota.resets_at) : "—";
    }
    function connectionExhaustionForecast(connection) {
        if (!connection || root.isRelayConnection(connection))
            return "";
        const quota = root.connectionQuota(connection);
        if (!quota)
            return "正在积累用量样本";
        const now = new Date();
        const start = quota.window_start ? new Date(quota.window_start) : null;
        const reset = quota.resets_at ? new Date(quota.resets_at) : (quota.window_end ? new Date(quota.window_end) : null);
        const remaining = quota.remaining_ratio === null || quota.remaining_ratio === undefined ? null : Number(quota.remaining_ratio);
        const used = remaining === null ? null : 1 - remaining;
        if (start && !isNaN(start.getTime()) && used !== null && used > 0 && now > start) {
            const exhaustsAt = new Date(start.getTime() + (now.getTime() - start.getTime()) / used);
            if (!reset || isNaN(reset.getTime()) || exhaustsAt < reset)
                return "预计耗尽 " + root.localDateTime(exhaustsAt);
            return "本周期预计不会耗尽";
        }
        const forecast = root.localCodex.weekly_quota_forecast;
        if (connection.connection_type === "chatgpt_subscription" && forecast)
            return forecast.exhausts_at ? "预计耗尽 " + root.localDateTime(forecast.exhausts_at) : "本周期预计不会耗尽";
        return "正在积累用量样本";
    }

    function selectConnection(connection) {
        if (!connection)
            return;
        root.selectedConnectionId = connection.id;
        root.updateSelectedConnectionView();
    }
    function connectionBudgetAmount(connection) {
        return (connection && connection.budget && connection.budget.amount !== null && connection.budget.amount !== undefined) ? String(connection.budget.amount) : "";
    }
    function isRelayConnection(connection) {
        return connection && connection.provider_id === "relay";
    }
    function isLocallyTrackedSubscription(connection) {
        return connection && connection.connection_type === "chatgpt_subscription";
    }

    function localSubscriptionTodayCost() {
        const value = root.localCodex.today_estimated_cost_usd;
        if (value === null || value === undefined)
            return "本地费用样本不足";
        const amount = Number(value || 0);
        return "本地 API 等价估算 $" + amount.toFixed(amount > 0 && amount < 1 ? 4 : 2);
    }
    function metricUnitKey(unit) {
        if (!unit)
            return "credit";
        if (typeof unit === "string")
            return unit;
        return unit.kind === "currency" ? String(unit.code || "USD").toUpperCase() : String(unit.kind || "credit");
    }
    function formatSpent(value, unit) {
        const amount = Number(value || 0);
        const digits = amount > 0 && amount < 1 ? 4 : 2;
        if (unit === "USD")
            return "$" + amount.toFixed(digits);
        if (unit === "credit")
            return amount.toFixed(digits) + " Credits";
        return amount.toFixed(digits) + " " + unit;
    }
    function isCurrentMetric(metric, now) {
        if (!metric || !metric.period_start)
            return false;
        const start = new Date(metric.period_start);
        const end = metric.period_end ? new Date(metric.period_end) : null;
        return !isNaN(start.getTime()) && start <= now && (!end || isNaN(end.getTime()) || now < end);
    }
    function accountMetrics(connection) {
        return ((connection && connection.metrics) || []).filter(metric => metric.scope === "account" || !metric.dimensions || !metric.dimensions.model);
    }
    function connectionTodaySpent(connection) {
        const now = new Date();
        const metrics = root.accountMetrics(connection).filter(metric => root.isCurrentMetric(metric, now));
        let selected = metrics.filter(metric => metric.metric_key === "cost.actual");
        if (!selected.length)
            selected = metrics.filter(metric => metric.metric_key === "credit.used");
        if (!selected.length)
            selected = metrics.filter(metric => metric.metric_key === "cost.estimated");
        if (!selected.length)
            return "今日暂无消费记录";
        const totals = {};
        selected.forEach(metric => {
            const unit = root.metricUnitKey(metric.unit);
            totals[unit] = Number(totals[unit] || 0) + Number(metric.value || 0);
        });
        return Object.keys(totals).map(unit => root.formatSpent(totals[unit], unit)).join(" + ");
    }

    function connectionTodayTokenValue(connection, key) {
        const now = new Date();
        return root.accountMetrics(connection).filter(metric => metric.metric_key === key && root.isCurrentMetric(metric, now)).reduce((sum, metric) => sum + Number(metric.value || 0), 0);
    }

    function connectionTodayTokens(connection) {
        const input = root.connectionTodayTokenValue(connection, "token.input");
        const cached = root.connectionTodayTokenValue(connection, "token.cached_input");
        const output = root.connectionTodayTokenValue(connection, "token.output");
        if (!input && !cached && !output)
            return "今日暂无 Token 记录";
        return "输入 " + root.compact(input) + " · 缓存 " + root.compact(cached) + " · 输出 " + root.compact(output);
    }

    function compact(value) {
        const number = Number(value || 0);
        if (Math.abs(number) >= 1000000)
            return (number / 1000000).toFixed(2) + "M";
        if (Math.abs(number) >= 1000)
            return (number / 1000).toFixed(1) + "K";
        return Math.round(number).toLocaleString();
    }

    function localDateTime(value) {
        if (!value)
            return "永久有效";
        const date = new Date(value);
        if (isNaN(date.getTime()))
            return "未知";
        return date.toLocaleString(Qt.locale(), "yyyy-MM-dd HH:mm");
    }

    function localDateKey(date) {
        const year = date.getFullYear();
        const month = String(date.getMonth() + 1).padStart(2, "0");
        const day = String(date.getDate()).padStart(2, "0");
        return year + "-" + month + "-" + day;
    }

    function metricDateKey(metric, now) {
        if (root.isCurrentMetric(metric, now))
            return root.localDateKey(now);
        const start = new Date(metric.period_start);
        return isNaN(start.getTime()) ? "" : root.localDateKey(start);
    }

    function relayTodayMetrics(items, key, now) {
        return items.filter(connection => root.isRelayConnection(connection)).flatMap(connection => root.accountMetrics(connection).filter(metric => metric.metric_key === key && root.isCurrentMetric(metric, now)));
    }

    function connectionPeriodValues(connection, keys, now) {
        const values = [0, 0, 0, 0, 0, 0, 0];
        if (!connection)
            return values;
        if (root.isLocallyTrackedSubscription(connection)) {
            (root.localCodex.daily_usage || []).forEach(usage => {
                const index = keys.indexOf(String(usage.date || ""));
                if (index >= 0)
                    values[index] = Number(usage.total_tokens || 0);
            });
            return values;
        }
        const metrics = root.accountMetrics(connection);
        const totals = metrics.filter(metric => metric.metric_key === "token.total" && metric.period_start);
        const source = totals.length ? totals : metrics.filter(metric => (metric.metric_key === "token.input" || metric.metric_key === "token.output") && metric.period_start);
        source.forEach(metric => {
            const index = keys.indexOf(root.metricDateKey(metric, now));
            if (index >= 0)
                values[index] += Number(metric.value || 0);
        });
        return values;
    }

    function updateSelectedConnectionView() {
        const connection = root.selectedConnection;
        if (!connection)
            return;
        root.accountName = connection.display_name || "LLM Meter";
        root.connectionStatus = root.statusText(connection.status);
        const selectedQuota = root.connectionQuota(connection);
        root.quotaPercent = selectedQuota ? Math.round(Number(selectedQuota.remaining_ratio) * 100) : -1;
        root.quotaName = selectedQuota ? (selectedQuota.display_name || "当前额度") : "当前额度";
        root.quotaResetsAt = selectedQuota && selectedQuota.resets_at ? root.localDateTime(selectedQuota.resets_at) : "—";
        root.quotaForecast = root.connectionExhaustionForecast(connection) || "";
        const now = new Date();
        const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
        const keys = [];
        for (let index = 6; index >= 0; index--) {
            const date = new Date(today);
            date.setDate(date.getDate() - index);
            keys.push(root.localDateKey(date));
        }
        const values = root.connectionPeriodValues(connection, keys, now);
        root.trendValues = values;
        root.trendMaximum = Math.max.apply(null, values.concat([1]));
        root.trendTotal = root.compact(values.reduce((sum, value) => sum + value, 0)) + " Token";
        root.todayTokens = root.compact(values[6]);
        if (root.isLocallyTrackedSubscription(connection)) {
            const value = root.localCodex.today_estimated_cost_usd;
            const hasCost = value !== null && value !== undefined;
            const cost = hasCost ? Number(value) : 0;
            root.todayCost = hasCost ? "$" + cost.toFixed(cost > 0 && cost < 1 ? 4 : 2) : "—";
            const models = new Set((root.localCodex.models || []).map(model => model.model));
            root.modelSummary = models.size ? Array.from(models).slice(0, 3).join(" · ") + (models.size > 3 ? " 等" : "") : "本地使用尚无模型样本";
        } else {
            const todayCost = root.accountMetrics(connection).filter(metric => metric.metric_key === "cost.actual" && root.isCurrentMetric(metric, now));
            const estimatedCost = todayCost.length ? [] : root.accountMetrics(connection).filter(metric => metric.metric_key === "cost.estimated" && root.isCurrentMetric(metric, now));
            const costs = todayCost.length ? todayCost : estimatedCost;
            const cost = costs.reduce((sum, metric) => sum + Number(metric.value || 0), 0);
            root.todayCost = costs.length ? "$" + cost.toFixed(cost > 0 && cost < 1 ? 4 : 2) : "—";
            const models = new Set();
            (connection.metrics || []).forEach(metric => {
                if (metric.dimensions && metric.dimensions.model)
                    models.add(metric.dimensions.model);
            });
            root.modelSummary = models.size ? Array.from(models).slice(0, 3).join(" · ") + (models.size > 3 ? " 等" : "") : "Provider 未返回模型维度";
        }
    }

    function resetStatusText(status) {
        const labels = {
            "available": "可使用",
            "redeeming": "使用中",
            "redeemed": "已使用",
            "unknown": "未知"
        };
        return labels[status] || status || "未知";
    }

    function consume(text) {
        try {
            const snapshot = JSON.parse(String(text));
            const items = snapshot.connections || [];
            root.connections = items;
            root.localCodex = snapshot.local_codex || ({
                    active_sessions: [],
                    models: []
                });
            root.offline = false;
            if (!items.length) {
                root.selectedConnectionId = "";
                root.accountName = "尚未添加连接";
                root.connectionStatus = "可在设置中登录";
                root.quotaPercent = -1;
                root.modelSummary = "登录后显示模型用量";
                return;
            }
            if (!items.some(connection => connection.id === root.selectedConnectionId))
                root.selectedConnectionId = items[0].id;
            root.updateSelectedConnectionView();
        } catch (error) {
            root.offline = true;
            root.connectionStatus = "daemon 离线";
        }
    }

    function refresh() {
        if (!snapshotProcess.running)
            snapshotProcess.running = true;
    }

    function refreshAutostart() {
        if (!autostartStatusProcess.running)
            autostartStatusProcess.running = true;
    }

    function runAction(arguments, pendingText) {
        if (root.busy)
            return;
        root.actionMessage = pendingText;
        actionProcess.command = [root.cli].concat(arguments);
        actionProcess.running = true;
    }

    function saveBarSetting(key, value) {
        if (!root.pluginApi?.pluginSettings)
            return;
        root.pluginApi.pluginSettings[key] = value;
        root.pluginApi.saveSettings();
    }

    Component.onCompleted: {
        refresh();
        refreshAutostart();
        refreshLoginProviders();
    }
    onActivePageChanged: {
        if (activePage === "settings")
            refreshAutostart();
    }
    onVisibleChanged: {
        if (visible) {
            root.activePage = "overview";
            root.actionMessage = "";
            root.pendingRemoveId = "";
            root.refresh();
            if (root.proxyOpen)
                root.refreshProxy();
        }
    }

    Process {
        id: snapshotProcess
        command: [root.cli, "status"]
        stdout: StdioCollector {}
        onExited: exitCode => {
            if (exitCode === 0)
                root.consume(snapshotProcess.stdout.text);
            else {
                root.offline = true;
                root.connectionStatus = "daemon 离线";
            }
        }
    }

    Process {
        id: actionProcess
        stdinEnabled: true
        onStarted: {
            if (root.pendingProcessSecret !== "") {
                actionProcess.write(root.pendingProcessSecret + "\n");
                root.pendingProcessSecret = "";
            }
        }
        stdout: StdioCollector {}
        stderr: StdioCollector {}
        onExited: exitCode => {
            root.pendingRemoveId = "";
            root.actionMessage = exitCode === 0 ? "操作成功，数据已刷新" : "操作失败：" + String(actionProcess.stderr.text).trim();
            if (actionProcess.command.length > 1 && actionProcess.command[1] === "autostart")
                root.refreshAutostart();
            root.refresh();
            if (root.proxyOpen)
                root.refreshProxy();
        }
    }

    Process {
        id: autostartStatusProcess
        command: [root.cli, "autostart", "status"]
        stdout: StdioCollector {}
        onExited: exitCode => {
            if (exitCode !== 0) {
                root.autostartKnown = false;
                return;
            }
            try {
                const status = JSON.parse(String(autostartStatusProcess.stdout.text));
                root.autostartEnabled = status.enabled === true;
                root.autostartActive = status.active === true;
                root.autostartKnown = true;
            } catch (error) {
                root.autostartKnown = false;
            }
        }
    }

    Process {
        id: loginProcess
        command: []
        stdout: StdioCollector {}
        stderr: StdioCollector {}
        onExited: exitCode => {
            if (root.loginCancelled)
                root.actionMessage = "已取消登录";
            else
                root.actionMessage = exitCode === 0 ? "登录成功，正在刷新用量…" : "登录未完成：" + String(loginProcess.stderr.text).trim();
            root.refresh();
        }
    }

    Process {
        id: providersProcess
        command: [root.cli, "diagnostics"]
        stdout: StdioCollector {}
        stderr: StdioCollector {}
        onExited: exitCode => {
            if (exitCode === 0)
                root.parseLoginOptions(providersProcess.stdout.text);
        }
    }

    Process {
        id: proxyStatusProcess
        command: []
        stdout: StdioCollector {}
        stderr: StdioCollector {}
        onExited: exitCode => {
            if (exitCode !== 0)
                return;
            try {
                const status = JSON.parse(String(proxyStatusProcess.stdout.text));
                root.proxyRunning = status.running === true;
                root.proxyListen = status.listen || "127.0.0.1:18456";
            } catch (error) {
                root.proxyRunning = false;
            }
        }
    }

    Process {
        id: proxyCredentialsProcess
        command: []
        stdout: StdioCollector {}
        stderr: StdioCollector {}
        onExited: exitCode => {
            if (exitCode !== 0)
                return;
            try {
                root.proxyCredentials = JSON.parse(String(proxyCredentialsProcess.stdout.text)).filter(credential => !credential.disabled_at);
            } catch (error) {
                root.proxyCredentials = [];
            }
        }
    }

    Process {
        id: credentialCreateProcess
        command: []
        stdout: StdioCollector {}
        stderr: StdioCollector {}
        onExited: exitCode => {
            if (exitCode === 0) {
                try {
                    root.createdProxyToken = JSON.parse(String(credentialCreateProcess.stdout.text)).token || "";
                    root.copyProxyToken(root.createdProxyToken);
                } catch (error) {
                    root.createdProxyToken = "";
                }
                root.proxyCredentialName = "Client";
                root.proxyCredentialToken = "";
                root.actionMessage = "客户端凭证已创建并复制到剪贴板";
                root.refreshProxy();
            } else {
                root.actionMessage = "操作失败：" + String(credentialCreateProcess.stderr.text).trim();
            }
        }
    }

    Process {
        id: clipboardProcess
        command: []
        stderr: StdioCollector {}
        onExited: exitCode => {
            if (exitCode !== 0)
                root.actionMessage = "客户端凭证已创建，但复制到剪贴板失败：" + String(clipboardProcess.stderr.text).trim();
        }
    }

    Timer {
        interval: 5000
        repeat: true
        running: root.visible && !root.busy
        onTriggered: root.refresh()
    }

    Rectangle {
        id: panelContainer
        anchors.fill: parent
        color: "transparent"

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: Style.marginL
            spacing: Style.marginM

            NBox {
                color: root.glassCardColor
                clip: true
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                Layout.preferredWidth: parent.width
                Layout.maximumWidth: parent.width
                implicitHeight: headerRow.implicitHeight + Style.margin2M
                RowLayout {
                    id: headerRow
                    anchors.fill: parent
                    anchors.margins: Style.marginM
                    spacing: Style.marginS
                    NIcon {
                        icon: root.activePage === "settings" ? "settings" : (root.activePage === "accounts" ? "users" : (root.activePage === "resets" ? "restore" : (root.activePage === "local" ? "terminal-2" : "chart-line")))
                        pointSize: Style.fontSizeXXL
                        color: root.offline ? Color.mError : Color.mPrimary
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.minimumWidth: 0
                        Layout.preferredWidth: 1
                        spacing: 0
                        NText {
                            text: root.activePage === "settings" ? "LLM Meter 设置" : (root.activePage === "accounts" ? "账户与 Provider" : (root.activePage === "resets" ? "Reset 额度" : (root.activePage === "local" ? "本地 Codex" : (root.connections.length > 1 ? "多账号概览" : root.accountName))))
                            pointSize: Style.fontSizeL
                            font.weight: Style.fontWeightBold
                            color: Color.mOnSurface
                            Layout.fillWidth: true
                            Layout.minimumWidth: 0
                            elide: Text.ElideRight
                        }
                        NText {
                            text: root.activePage === "settings" ? "应用与顶栏偏好" : (root.activePage === "accounts" ? "登录、Relay 与本地代理" : (root.activePage === "resets" ? "ChatGPT 订阅重置机会" : (root.activePage === "local" ? "运行会话、模型与 API 等价费用" : (root.connections.length > 1 ? root.connections.length + " 个账号" : root.connectionStatus))))
                            pointSize: Style.fontSizeXS
                            color: root.offline ? Color.mError : Color.mOnSurfaceVariant
                            Layout.fillWidth: true
                            Layout.minimumWidth: 0
                            elide: Text.ElideRight
                        }
                    }
                    NIconButton {
                        visible: root.activePage === "overview" && root.hasSubscription
                        icon: "restore"
                        tooltipText: "Reset 额度"
                        baseSize: Style.baseWidgetSize * 0.72
                        onClicked: {
                            root.activePage = "resets";
                            root.actionMessage = "";
                            root.pendingRemoveId = "";
                        }
                    }
                    NIconButton {
                        visible: root.activePage === "overview"
                        icon: "terminal-2"
                        tooltipText: "本地 Codex"
                        baseSize: Style.baseWidgetSize * 0.72
                        onClicked: {
                            root.activePage = "local";
                            root.actionMessage = "";
                        }
                    }
                    NIconButton {
                        visible: root.activePage === "overview"
                        icon: "users"
                        tooltipText: "账户管理"
                        baseSize: Style.baseWidgetSize * 0.72
                        onClicked: {
                            root.activePage = "accounts";
                            root.actionMessage = "";
                            root.pendingRemoveId = "";
                        }
                    }
                    NIconButton {
                        icon: root.activePage === "overview" ? "settings" : "arrow-left"
                        tooltipText: root.activePage === "overview" ? "设置" : "返回概览"
                        baseSize: Style.baseWidgetSize * 0.72
                        onClicked: {
                            root.activePage = root.activePage === "overview" ? "settings" : "overview";
                            root.actionMessage = "";
                            root.pendingRemoveId = "";
                        }
                    }
                    NIconButton {
                        icon: "refresh"
                        tooltipText: "强制刷新全部数据"
                        baseSize: Style.baseWidgetSize * 0.72
                        enabled: !root.busy
                        onClicked: root.runAction(["refresh-all"], "正在刷新全部连接与本地 Codex…")
                    }
                    NIconButton {
                        icon: "close"
                        tooltipText: "关闭"
                        baseSize: Style.baseWidgetSize * 0.72
                        onClicked: pluginApi?.closePanel(pluginApi?.panelOpenScreen)
                    }
                }
            }

            Loader {
                Layout.fillWidth: true
                Layout.fillHeight: true
                sourceComponent: root.activePage === "settings" ? settingsPage : (root.activePage === "accounts" ? accountsPage : (root.activePage === "resets" ? resetsPage : (root.activePage === "local" ? localCodexPage : overviewPage)))
            }
        }
    }

    Component {
        id: localCodexPage
        Flickable {
            clip: true
            contentWidth: width
            contentHeight: localCodexColumn.implicitHeight
            boundsBehavior: Flickable.StopAtBounds

            ColumnLayout {
                id: localCodexColumn
                width: parent.width
                spacing: Style.marginM

                NBox {
                    color: root.glassCardColor
                    Layout.fillWidth: true
                    implicitHeight: localSummary.implicitHeight + Style.margin2M
                    ColumnLayout {
                        id: localSummary
                        anchors.fill: parent
                        anchors.margins: Style.marginM
                        spacing: Style.marginXS
                        RowLayout {
                            Layout.fillWidth: true
                            NIcon {
                                icon: "terminal-2"
                                color: Color.mPrimary
                                pointSize: Style.fontSizeXL
                            }
                            NText {
                                text: (root.localCodex.active_sessions || []).length + " 个运行中会话"
                                color: Color.mOnSurface
                                font.weight: Style.fontWeightBold
                                Layout.fillWidth: true
                            }
                            NText {
                                text: root.localCodex.estimated_cost_usd !== null && root.localCodex.estimated_cost_usd !== undefined ? "$" + Number(root.localCodex.estimated_cost_usd).toFixed(2) : "—"
                                color: Color.mPrimary
                                pointSize: Style.fontSizeXL
                                font.weight: Style.fontWeightBold
                            }
                        }
                        NText {
                            text: "当前运行会话累计的 API 等价费用估算；ChatGPT 订阅不会按此金额额外收费"
                            color: Color.mOnSurfaceVariant
                            pointSize: Style.fontSizeXS
                            wrapMode: Text.WordWrap
                            Layout.fillWidth: true
                        }
                    }
                }

                Repeater {
                    model: root.localCodex.models || []
                    NBox {
                        required property var modelData
                        color: root.glassCardColor
                        Layout.fillWidth: true
                        implicitHeight: modelUsageColumn.implicitHeight + Style.margin2M
                        ColumnLayout {
                            id: modelUsageColumn
                            anchors.fill: parent
                            anchors.margins: Style.marginM
                            spacing: Style.marginXS
                            RowLayout {
                                Layout.fillWidth: true
                                NText {
                                    text: modelData.model
                                    color: Color.mOnSurface
                                    font.weight: Style.fontWeightBold
                                    Layout.fillWidth: true
                                    elide: Text.ElideRight
                                }
                                NText {
                                    text: modelData.estimated_cost_usd !== null && modelData.estimated_cost_usd !== undefined ? "$" + Number(modelData.estimated_cost_usd).toFixed(2) : "价格未知"
                                    color: Color.mPrimary
                                }
                            }
                            NText {
                                text: "输入 " + root.compact(modelData.input_tokens) + " · 缓存 " + root.compact(modelData.cached_input_tokens) + " · 输出 " + root.compact(modelData.output_tokens)
                                color: Color.mOnSurfaceVariant
                                pointSize: Style.fontSizeXS
                                Layout.fillWidth: true
                            }
                        }
                    }
                }

                Repeater {
                    model: root.localCodex.active_sessions || []
                    NBox {
                        required property var modelData
                        color: root.glassCardColor
                        Layout.fillWidth: true
                        implicitHeight: sessionColumn.implicitHeight + Style.margin2M
                        ColumnLayout {
                            id: sessionColumn
                            anchors.fill: parent
                            anchors.margins: Style.marginM
                            spacing: Style.marginXS
                            NText {
                                text: modelData.model + " · " + root.compact(modelData.total_tokens) + " Token"
                                color: Color.mOnSurface
                                font.weight: Style.fontWeightSemiBold
                                Layout.fillWidth: true
                            }
                            NText {
                                text: modelData.cwd || modelData.id
                                color: Color.mOnSurfaceVariant
                                pointSize: Style.fontSizeXS
                                elide: Text.ElideMiddle
                                Layout.fillWidth: true
                            }
                        }
                    }
                }

                NText {
                    text: "官方标准处理价格 · 更新于 " + (root.localCodex.pricing_as_of || "未知")
                    color: Color.mOnSurfaceVariant
                    pointSize: Style.fontSizeXS
                    horizontalAlignment: Text.AlignHCenter
                    Layout.fillWidth: true
                }
            }
        }
    }

    Component {
        id: resetsPage
        Flickable {
            clip: true
            contentWidth: width
            contentHeight: resetsColumn.implicitHeight
            boundsBehavior: Flickable.StopAtBounds

            ColumnLayout {
                id: resetsColumn
                width: parent.width
                spacing: Style.marginM

                NBox {
                    color: root.glassCardColor
                    Layout.fillWidth: true
                    implicitHeight: resetSummaryColumn.implicitHeight + Style.margin2M
                    ColumnLayout {
                        id: resetSummaryColumn
                        anchors.fill: parent
                        anchors.margins: Style.marginM
                        spacing: Style.marginXS
                        RowLayout {
                            Layout.fillWidth: true
                            NIcon {
                                icon: "restore"
                                color: Color.mPrimary
                                pointSize: Style.fontSizeXL
                            }
                            NText {
                                text: "剩余 Reset 机会"
                                color: Color.mOnSurfaceVariant
                                Layout.fillWidth: true
                            }
                            NText {
                                text: root.resetSummaries.length ? root.resetAvailableCount + " 次" : "—"
                                color: Color.mPrimary
                                pointSize: Style.fontSizeXXL
                                font.weight: Style.fontWeightBold
                            }
                        }
                        NText {
                            text: root.resetSummaries.length ? (root.resetDetailsKnown ? "以下时间均为本地时间" : "Provider 仅返回总次数，暂未提供逐条过期时间") : "尚未获取 Reset 额度，请刷新订阅连接"
                            color: Color.mOnSurfaceVariant
                            pointSize: Style.fontSizeXS
                            wrapMode: Text.WordWrap
                            Layout.fillWidth: true
                        }
                    }
                }

                Repeater {
                    model: root.resetCreditItems
                    NBox {
                        required property var modelData
                        color: root.glassCardColor
                        Layout.fillWidth: true
                        implicitHeight: resetCreditColumn.implicitHeight + Style.margin2M
                        ColumnLayout {
                            id: resetCreditColumn
                            anchors.fill: parent
                            anchors.margins: Style.marginM
                            spacing: Style.marginXS
                            RowLayout {
                                Layout.fillWidth: true
                                NText {
                                    text: modelData.title || "Full reset"
                                    color: Color.mOnSurface
                                    font.weight: Style.fontWeightSemiBold
                                    Layout.fillWidth: true
                                    elide: Text.ElideRight
                                }
                                NText {
                                    text: root.resetStatusText(modelData.status)
                                    color: modelData.status === "available" ? Color.mPrimary : Color.mOnSurfaceVariant
                                    pointSize: Style.fontSizeXS
                                }
                            }
                            RowLayout {
                                Layout.fillWidth: true
                                NIcon {
                                    icon: "calendar-clock"
                                    color: Color.mPrimary
                                    pointSize: Style.fontSizeM
                                }
                                NText {
                                    text: "过期"
                                    color: Color.mOnSurfaceVariant
                                    pointSize: Style.fontSizeXS
                                }
                                NText {
                                    text: root.localDateTime(modelData.expires_at)
                                    color: Color.mOnSurface
                                    font.weight: Style.fontWeightSemiBold
                                    Layout.fillWidth: true
                                    horizontalAlignment: Text.AlignRight
                                }
                            }
                        }
                    }
                }

                NBox {
                    visible: root.resetSummaries.length > 0 && root.resetDetailsKnown && root.resetCreditItems.length < root.resetAvailableCount
                    color: root.glassCardColor
                    Layout.fillWidth: true
                    implicitHeight: partialDetailsText.implicitHeight + Style.margin2M
                    NText {
                        id: partialDetailsText
                        anchors.fill: parent
                        anchors.margins: Style.marginM
                        text: "服务端返回了 " + root.resetAvailableCount + " 次机会，但仅提供 " + root.resetCreditItems.length + " 条明细。"
                        color: Color.mOnSurfaceVariant
                        pointSize: Style.fontSizeXS
                        wrapMode: Text.WordWrap
                    }
                }
            }
        }
    }

    Component {
        id: overviewPage
        Flickable {
            clip: true
            contentWidth: width
            contentHeight: overviewColumn.implicitHeight
            boundsBehavior: Flickable.StopAtBounds

            ColumnLayout {
                id: overviewColumn
                width: parent.width
                spacing: Style.marginM

                NBox {
                    visible: root.connections.length > 0
                    color: root.glassCardColor
                    Layout.fillWidth: true
                    implicitHeight: accountsColumn.implicitHeight + Style.margin2M
                    ColumnLayout {
                        id: accountsColumn
                        anchors.fill: parent
                        anchors.margins: Style.marginM
                        spacing: Style.marginS
                        RowLayout {
                            Layout.fillWidth: true
                            NText {
                                text: "账号"
                                font.weight: Style.fontWeightBold
                                color: Color.mOnSurface
                                Layout.fillWidth: true
                            }
                            NText {
                                text: root.connections.length + " 个"
                                pointSize: Style.fontSizeXS
                                color: Color.mOnSurfaceVariant
                            }
                        }

                        Repeater {
                            model: root.connections
                            NBox {
                                required property var modelData
                                color: root.selectedConnectionId === modelData.id ? Qt.alpha(Color.mPrimary, 0.16) : Qt.alpha(Color.mSurface, 0.28)
                                Layout.fillWidth: true
                                implicitHeight: accountRow.implicitHeight + Style.marginM
                                RowLayout {
                                    id: accountRow
                                    anchors.fill: parent
                                    anchors.margins: Style.marginS
                                    spacing: Style.marginS
                                    NIcon {
                                        icon: root.providerIcon(modelData.provider_id)
                                        color: Color.mPrimary
                                        pointSize: Style.fontSizeXL
                                    }
                                    ColumnLayout {
                                        Layout.fillWidth: true
                                        spacing: 0
                                        RowLayout {
                                            Layout.fillWidth: true
                                            NText {
                                                text: modelData.display_name || "OpenAI"
                                                color: Color.mOnSurface
                                                font.weight: Style.fontWeightSemiBold
                                                elide: Text.ElideRight
                                                Layout.fillWidth: true
                                            }
                                            NText {
                                                text: root.statusText(modelData.status)
                                                color: root.statusColor(modelData.status)
                                                pointSize: Style.fontSizeXS
                                                font.weight: Style.fontWeightSemiBold
                                            }
                                        }
                                        RowLayout {
                                            Layout.fillWidth: true
                                            NText {
                                                text: root.isRelayConnection(modelData) ? "今日已消费" : root.connectionQuotaName(modelData)
                                                color: Color.mOnSurfaceVariant
                                                pointSize: Style.fontSizeXS
                                                Layout.fillWidth: true
                                            }
                                            NText {
                                                text: root.isRelayConnection(modelData) ? root.connectionTodaySpent(modelData) : (root.connectionQuotaPercent(modelData) >= 0 ? root.connectionQuotaPercent(modelData) + "% 剩余" : "额度未提供")
                                                color: Color.mOnSurface
                                                pointSize: Style.fontSizeXS
                                            }
                                        }
                                        NText {
                                            visible: root.isLocallyTrackedSubscription(modelData)
                                            text: root.localSubscriptionTodayCost()
                                            color: Color.mOnSurfaceVariant
                                            pointSize: Style.fontSizeXS
                                            Layout.fillWidth: true
                                        }
                                        NText {
                                            visible: root.isRelayConnection(modelData)
                                            text: root.connectionTodayTokens(modelData)
                                            color: Color.mOnSurfaceVariant
                                            pointSize: Style.fontSizeXS
                                            Layout.fillWidth: true
                                            wrapMode: Text.WordWrap
                                        }
                                        NLinearGauge {
                                            visible: !root.isRelayConnection(modelData)
                                            Layout.fillWidth: true
                                            Layout.preferredHeight: visible ? Math.max(4, Math.round(4 * Style.uiScaleRatio)) : 0
                                            orientation: Qt.Horizontal
                                            ratio: root.connectionQuotaPercent(modelData) < 0 ? 0 : root.connectionQuotaPercent(modelData) / 100
                                            fillColor: root.connectionQuotaPercent(modelData) >= 0 && root.connectionQuotaPercent(modelData) < 20 ? Color.mError : Color.mPrimary
                                        }
                                        RowLayout {
                                            visible: !root.isRelayConnection(modelData)
                                            Layout.fillWidth: true
                                            NIcon {
                                                icon: "calendar-clock"
                                                color: Color.mPrimary
                                                pointSize: Style.fontSizeXS
                                            }
                                            NText {
                                                text: "下次重置 " + root.connectionQuotaResetsAt(modelData)
                                                color: Color.mOnSurfaceVariant
                                                pointSize: Style.fontSizeXS
                                                Layout.fillWidth: true
                                            }
                                            NText {
                                                text: root.connectionExhaustionForecast(modelData)
                                                color: Color.mOnSurfaceVariant
                                                pointSize: Style.fontSizeXS
                                                horizontalAlignment: Text.AlignRight
                                            }
                                        }
                                    }
                                }
                                TapHandler {
                                    onTapped: root.selectConnection(modelData)
                                }
                            }
                        }

                        NText {
                            visible: root.connections.length === 0
                            text: "尚未连接账号，可在设置中登录。"
                            wrapMode: Text.WordWrap
                            Layout.fillWidth: true
                            color: Color.mOnSurfaceVariant
                            pointSize: Style.fontSizeXS
                        }
                    }
                }

                NBox {
                    visible: root.nearestResetCredit !== null
                    color: root.nearestResetUrgent ? Qt.alpha(Color.mError, 0.16) : root.glassCardColor
                    Layout.fillWidth: true
                    implicitHeight: nearestResetRow.implicitHeight + Style.margin2M
                    RowLayout {
                        id: nearestResetRow
                        anchors.fill: parent
                        anchors.margins: Style.marginM
                        spacing: Style.marginS
                        NIcon {
                            icon: "restore"
                            color: root.nearestResetUrgent ? Color.mError : Color.mPrimary
                            pointSize: Style.fontSizeL
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            NText {
                                text: "最近到期的 Reset 机会"
                                color: Color.mOnSurfaceVariant
                                pointSize: Style.fontSizeXS
                            }
                            NText {
                                text: root.localDateTime(root.nearestResetCredit ? root.nearestResetCredit.expires_at : null)
                                color: root.nearestResetUrgent ? Color.mError : Color.mOnSurface
                                font.weight: Style.fontWeightBold
                            }
                        }
                        NText {
                            text: root.resetAvailableCount + " 次可用"
                            color: Color.mPrimary
                            pointSize: Style.fontSizeXS
                        }
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Style.marginM
                    Repeater {
                        model: [
                            {
                                icon: "coins",
                                label: "今日 Token",
                                value: root.todayTokens
                            },
                            {
                                icon: "currency-dollar",
                                label: "今日费用",
                                value: root.todayCost
                            }
                        ]
                        NBox {
                            color: root.glassCardColor
                            Layout.fillWidth: true
                            implicitHeight: statColumn.implicitHeight + Style.margin2M
                            ColumnLayout {
                                id: statColumn
                                anchors.fill: parent
                                anchors.margins: Style.marginM
                                spacing: Style.marginXS
                                RowLayout {
                                    NIcon {
                                        icon: modelData.icon
                                        color: Color.mPrimary
                                        pointSize: Style.fontSizeM
                                    }
                                    NText {
                                        text: modelData.label
                                        color: Color.mOnSurfaceVariant
                                        pointSize: Style.fontSizeXS
                                    }
                                }
                                NText {
                                    text: modelData.value
                                    color: Color.mOnSurface
                                    pointSize: Style.fontSizeXL
                                    font.weight: Style.fontWeightBold
                                }
                            }
                        }
                    }
                }

                NBox {
                    color: root.glassCardColor
                    Layout.fillWidth: true
                    Layout.preferredHeight: Math.round(180 * Style.uiScaleRatio)
                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: Style.marginM
                        spacing: Style.marginXS
                        RowLayout {
                            Layout.fillWidth: true
                            NText {
                                text: "近 7 天 Token 趋势 · " + (root.selectedConnection ? root.selectedConnection.display_name : "未选择账号")
                                color: Color.mOnSurfaceVariant
                                Layout.fillWidth: true
                            }
                            NText {
                                text: root.trendTotal
                                color: Color.mPrimary
                                font.weight: Style.fontWeightSemiBold
                            }
                        }
                        NGraph {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            Layout.preferredHeight: Math.round(120 * Style.uiScaleRatio)
                            Layout.minimumHeight: Math.round(96 * Style.uiScaleRatio)
                            values: root.trendValues
                            minValue: 0
                            maxValue: root.trendMaximum
                            color: Color.mPrimary
                            strokeWidth: Math.max(1, Style.uiScaleRatio)
                            fill: true
                            fillOpacity: 0.15
                            updateInterval: 0
                        }
                    }
                }
            }
        }
    }

    Component {
        id: settingsPage
        Flickable {
            clip: true
            contentWidth: width
            contentHeight: settingsColumn.implicitHeight
            boundsBehavior: Flickable.StopAtBounds

            ColumnLayout {
                id: settingsColumn
                width: parent.width
                spacing: Style.marginM

                NBox {
                    color: root.glassCardColor
                    Layout.fillWidth: true
                    implicitHeight: applicationSettingsColumn.implicitHeight + Style.margin2M
                    ColumnLayout {
                        id: applicationSettingsColumn
                        anchors.fill: parent
                        anchors.margins: Style.marginM
                        spacing: Style.marginS
                        NText {
                            text: "应用"
                            font.weight: Style.fontWeightBold
                            color: Color.mOnSurface
                        }
                        NToggle {
                            label: "开机自动启动"
                            description: root.autostartKnown ? (root.autostartActive ? "daemon 已随当前用户会话运行" : "登录桌面后自动启动 daemon") : "正在读取 systemd 用户服务状态"
                            checked: root.autostartEnabled
                            enabled: root.autostartKnown && !root.busy
                            onToggled: checked => {
                                root.autostartEnabled = checked;
                                root.runAction(["autostart", checked ? "enable" : "disable"], checked ? "正在启用开机自启动…" : "正在关闭开机自启动…");
                            }
                        }
                        NButton {
                            Layout.fillWidth: true
                            text: "更新到最新稳定版"
                            icon: "download"
                            outlined: true
                            enabled: !root.busy
                            onClicked: root.runAction(["update"], "正在从 crates.io 更新，请稍候…")
                        }
                    }
                }

                NBox {
                    color: root.glassCardColor
                    Layout.fillWidth: true
                    implicitHeight: barSettingsColumn.implicitHeight + Style.margin2M
                    ColumnLayout {
                        id: barSettingsColumn
                        anchors.fill: parent
                        anchors.margins: Style.marginM
                        spacing: Style.marginS
                        NText {
                            text: "顶栏显示"
                            font.weight: Style.fontWeightBold
                            color: Color.mOnSurface
                        }
                        NText {
                            text: "选择顶栏中需要显示的信息，修改后立即生效。多个账号时，顶栏会自动显示剩余额度最低的账号。"
                            color: Color.mOnSurfaceVariant
                            pointSize: Style.fontSizeXS
                            wrapMode: Text.WordWrap
                            Layout.fillWidth: true
                        }
                        NToggle {
                            label: "账号"
                            description: "显示主账号名称（多个账号时显示剩余额度最低的账号）"
                            checked: root.pluginApi?.pluginSettings?.barShowAccount ?? true
                            defaultValue: true
                            onToggled: checked => root.saveBarSetting("barShowAccount", checked)
                        }
                        NToggle {
                            label: "周额度"
                            description: "显示所有账号中最低的剩余额度百分比"
                            checked: root.pluginApi?.pluginSettings?.barShowQuota ?? true
                            defaultValue: true
                            onToggled: checked => root.saveBarSetting("barShowQuota", checked)
                        }
                        NToggle {
                            label: "今日 Token"
                            description: "显示本地 Codex 今日 Token"
                            checked: root.pluginApi?.pluginSettings?.barShowTodayTokens ?? false
                            defaultValue: false
                            onToggled: checked => root.saveBarSetting("barShowTodayTokens", checked)
                        }
                        NToggle {
                            label: "今日费用"
                            description: "显示今日 API 等价费用"
                            checked: root.pluginApi?.pluginSettings?.barShowTodayCost ?? false
                            defaultValue: false
                            onToggled: checked => root.saveBarSetting("barShowTodayCost", checked)
                        }
                        NToggle {
                            label: "Codex 会话数"
                            description: "显示当前运行中的 Codex 数量"
                            checked: root.pluginApi?.pluginSettings?.barShowCodexSessions ?? false
                            defaultValue: false
                            onToggled: checked => root.saveBarSetting("barShowCodexSessions", checked)
                        }
                        NToggle {
                            label: "Token 趋势"
                            description: "显示最近七天的紧凑趋势图"
                            checked: root.pluginApi?.pluginSettings?.barShowTrend ?? true
                            defaultValue: true
                            onToggled: checked => root.saveBarSetting("barShowTrend", checked)
                        }
                    }
                }

                NText {
                    visible: root.actionMessage !== ""
                    text: root.actionMessage
                    color: root.actionMessage.indexOf("失败") >= 0 || root.actionMessage.indexOf("未完成") >= 0 ? Color.mError : Color.mOnSurfaceVariant
                    pointSize: Style.fontSizeXS
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }
            }
        }
    }

    Component {
        id: accountsPage
        Flickable {
            clip: true
            contentWidth: width
            contentHeight: accountsColumn.implicitHeight
            boundsBehavior: Flickable.StopAtBounds

            ColumnLayout {
                id: accountsColumn
                width: parent.width
                spacing: Style.marginM

                NBox {
                    color: root.glassCardColor
                    Layout.fillWidth: true
                    implicitHeight: accountHeader.implicitHeight + Style.margin2M
                    RowLayout {
                        id: accountHeader
                        anchors.fill: parent
                        anchors.margins: Style.marginM
                        spacing: Style.marginS
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            NText {
                                text: "账户与 Provider"
                                font.weight: Style.fontWeightBold
                                color: Color.mOnSurface
                            }
                            NText {
                                text: "管理订阅、Platform API 和 OpenAI 兼容中转站"
                                color: Color.mOnSurfaceVariant
                                pointSize: Style.fontSizeXS
                            }
                        }
                        NButton {
                            text: "添加"
                            icon: "plus"
                            enabled: !root.busy
                            onClicked: root.openAddAccount()
                        }
                    }
                }

                NText {
                    visible: root.connections.length === 0
                    text: "尚未添加账户。支持 ChatGPT、Kimi、OpenAI Platform 与任意 OpenAI 兼容 Provider。"
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                    color: Color.mOnSurfaceVariant
                }

                Repeater {
                    model: root.connections
                    NBox {
                        required property var modelData
                        color: root.selectedConnectionId === modelData.id ? Qt.alpha(Color.mPrimary, 0.16) : root.glassCardColor
                        Layout.fillWidth: true
                        implicitHeight: accountCard.implicitHeight + Style.margin2M
                        ColumnLayout {
                            id: accountCard
                            anchors.fill: parent
                            anchors.margins: Style.marginM
                            spacing: Style.marginS
                            RowLayout {
                                Layout.fillWidth: true
                                NIcon {
                                    icon: root.providerIcon(modelData.provider_id)
                                    color: Color.mPrimary
                                    pointSize: Style.fontSizeXL
                                }
                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 0
                                    NText {
                                        text: modelData.display_name || "Provider"
                                        color: Color.mOnSurface
                                        font.weight: Style.fontWeightSemiBold
                                        elide: Text.ElideRight
                                        Layout.fillWidth: true
                                    }
                                    NText {
                                        text: modelData.provider_id + " · " + root.statusText(modelData.status)
                                        color: root.statusColor(modelData.status)
                                        pointSize: Style.fontSizeXS
                                    }
                                }
                                NIconButton {
                                    icon: "refresh"
                                    tooltipText: "刷新连接"
                                    enabled: !root.busy
                                    onClicked: root.runAction(["refresh", modelData.id], "正在刷新连接…")
                                }
                                NIconButton {
                                    icon: root.pendingRemoveId === modelData.id ? "check" : "trash"
                                    tooltipText: root.pendingRemoveId === modelData.id ? "再次点击确认删除" : "删除连接"
                                    colorFg: root.pendingRemoveId === modelData.id ? Color.mError : Color.mPrimary
                                    enabled: !root.busy
                                    onClicked: {
                                        if (root.pendingRemoveId === modelData.id)
                                            root.runAction(["remove", modelData.id], "正在删除连接…");
                                        else {
                                            root.pendingRemoveId = modelData.id;
                                            root.actionMessage = "再次点击红色按钮确认删除";
                                        }
                                    }
                                }
                            }
                            NText {
                                text: modelData.connection_type
                                color: Color.mOnSurfaceVariant
                                pointSize: Style.fontSizeXS
                            }
                            RowLayout {
                                Layout.fillWidth: true
                                NText {
                                    text: root.isRelayConnection(modelData) ? "今日已消费" : root.connectionQuotaName(modelData)
                                    color: Color.mOnSurfaceVariant
                                    Layout.fillWidth: true
                                    pointSize: Style.fontSizeXS
                                }
                                NText {
                                    text: root.isRelayConnection(modelData) ? root.connectionTodaySpent(modelData) : (root.connectionQuotaPercent(modelData) >= 0 ? root.connectionQuotaPercent(modelData) + "% 剩余" : "额度未提供")
                                    color: Color.mOnSurface
                                    pointSize: Style.fontSizeXS
                                }
                            }
                            NText {
                                visible: root.isLocallyTrackedSubscription(modelData)
                                text: root.localSubscriptionTodayCost()
                                color: Color.mOnSurfaceVariant
                                pointSize: Style.fontSizeXS
                                Layout.fillWidth: true
                            }
                            NText {
                                visible: root.isRelayConnection(modelData)
                                text: root.connectionTodayTokens(modelData)
                                color: Color.mOnSurfaceVariant
                                pointSize: Style.fontSizeXS
                                Layout.fillWidth: true
                                wrapMode: Text.WordWrap
                            }
                            RowLayout {
                                visible: !root.isRelayConnection(modelData)
                                Layout.fillWidth: true
                                NText {
                                    text: "下次重置 " + root.connectionQuotaResetsAt(modelData)
                                    color: Color.mOnSurfaceVariant
                                    pointSize: Style.fontSizeXS
                                    Layout.fillWidth: true
                                }
                                NText {
                                    text: root.connectionExhaustionForecast(modelData)
                                    color: Color.mOnSurfaceVariant
                                    pointSize: Style.fontSizeXS
                                }
                            }
                            NLinearGauge {
                                visible: !root.isRelayConnection(modelData)
                                Layout.fillWidth: true
                                Layout.preferredHeight: visible ? Math.max(4, Math.round(4 * Style.uiScaleRatio)) : 0
                                orientation: Qt.Horizontal
                                ratio: root.connectionQuotaPercent(modelData) < 0 ? 0 : root.connectionQuotaPercent(modelData) / 100
                                fillColor: root.connectionQuotaPercent(modelData) >= 0 && root.connectionQuotaPercent(modelData) < 20 ? Color.mError : Color.mPrimary
                            }
                            NButton {
                                visible: modelData.connection_type === "openai_compatible_proxy"
                                Layout.fillWidth: true
                                text: "配置本地代理"
                                icon: "server"
                                outlined: true
                                onClicked: root.openProxy(modelData)
                            }
                            RowLayout {
                                visible: modelData.connection_type !== "chatgpt_subscription" && modelData.connection_type !== "kimi_code_subscription"
                                Layout.fillWidth: true
                                NTextInput {
                                    id: accountBudgetInput
                                    Layout.fillWidth: true
                                    placeholderText: "月度预算 USD"
                                    text: root.connectionBudgetAmount(modelData)
                                    inputMethodHints: Qt.ImhFormattedNumbersOnly
                                    showClearButton: false
                                }
                                NButton {
                                    text: "保存预算"
                                    outlined: true
                                    enabled: !root.busy && accountBudgetInput.text.trim() !== ""
                                    onClicked: root.runAction(["budget", modelData.id, accountBudgetInput.text, "--currency", "USD"], "正在保存预算…")
                                }
                            }
                        }
                        TapHandler {
                            onTapped: root.selectConnection(modelData)
                        }
                    }
                }

                NText {
                    visible: root.actionMessage !== ""
                    text: root.actionMessage
                    color: root.actionMessage.indexOf("失败") >= 0 ? Color.mError : Color.mOnSurfaceVariant
                    pointSize: Style.fontSizeXS
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }
            }
        }
    }

    Rectangle {
        id: addAccountModal
        anchors.fill: parent
        visible: root.addAccountOpen
        color: Qt.rgba(0, 0, 0, 0.42)
        z: 100

        MouseArea {
            anchors.fill: parent
            onClicked: root.closeAddAccount()
        }

        NBox {
            anchors.centerIn: parent
            width: Math.min(parent.width - 2 * Style.marginL, 520 * Style.uiScaleRatio)
            height: modalColumn.implicitHeight + 2 * Style.marginM
            color: root.panelBackgroundColor

            ColumnLayout {
                id: modalColumn
                anchors.fill: parent
                anchors.margins: Style.marginM
                spacing: Style.marginM

                RowLayout {
                    Layout.fillWidth: true
                    NText {
                        text: "添加账号"
                        font.weight: Style.fontWeightBold
                        color: Color.mOnSurface
                        Layout.fillWidth: true
                    }
                    NIconButton {
                        icon: "close"
                        tooltipText: "关闭"
                        baseSize: Style.baseWidgetSize * 0.72
                        enabled: !root.busy
                        onClicked: root.closeAddAccount()
                    }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: Style.marginS
                    visible: !root.apiKeyMode

                    NText {
                        text: "选择要登录的账号类型"
                        color: Color.mOnSurfaceVariant
                        pointSize: Style.fontSizeXS
                        wrapMode: Text.WordWrap
                        Layout.fillWidth: true
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: Style.marginS
                        Repeater {
                            model: root.loginOptions
                            NButton {
                                Layout.fillWidth: true
                                text: modelData.label
                                icon: modelData.is_oauth ? "login" : "key"
                                outlined: true
                                horizontalAlignment: Qt.AlignLeft
                                enabled: !root.busy
                                onClicked: root.selectLoginOption(modelData)
                            }
                        }
                    }

                    NText {
                        text: "未检测到可登录的 Provider，请检查 daemon 是否已启动或版本是否匹配。"
                        visible: root.loginOptions.length === 0
                        wrapMode: Text.WordWrap
                        Layout.fillWidth: true
                        color: Color.mError
                        pointSize: Style.fontSizeXS
                    }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: Style.marginS
                    visible: root.apiKeyMode

                    NText {
                        text: root.relayMode ? "OpenAI 兼容 Provider" : (root.selectedLoginOption ? root.selectedLoginOption.label : "")
                        font.weight: Style.fontWeightBold
                        color: Color.mOnSurface
                    }
                    NText {
                        text: root.relayMode ? "输入 Provider 名称、Base URL 和上游 API Key。系统会验证 /v1/models，然后创建本地代理。" : "请输入 API Key，提交后会保存到系统密钥环。"
                        color: Color.mOnSurfaceVariant
                        pointSize: Style.fontSizeXS
                        wrapMode: Text.WordWrap
                        Layout.fillWidth: true
                    }
                    NTextInput {
                        visible: root.relayMode
                        Layout.fillWidth: true
                        placeholderText: "Provider 名称"
                        text: root.relayDisplayName
                        onTextChanged: root.relayDisplayName = text
                    }
                    NTextInput {
                        visible: root.relayMode
                        Layout.fillWidth: true
                        placeholderText: "https://api.example.com"
                        text: root.relayBaseUrl
                        onTextChanged: root.relayBaseUrl = text
                    }
                    NTextInput {
                        visible: root.relayMode
                        Layout.fillWidth: true
                        placeholderText: "127.0.0.1:18456"
                        text: root.relayListen
                        onTextChanged: root.relayListen = text
                    }
                    NTextInput {
                        id: apiKeyInputField
                        Layout.fillWidth: true
                        placeholderText: "sk-..."
                        text: root.apiKeyInput
                        inputItem.echoMode: TextInput.Password
                        onTextChanged: root.apiKeyInput = text
                    }
                    NText {
                        visible: root.relayMode
                        text: "连接成功后可直接在 Noctalia 中启动代理并管理多个客户端凭证。"
                        color: Color.mOnSurfaceVariant
                        pointSize: Style.fontSizeXS
                        wrapMode: Text.WordWrap
                        Layout.fillWidth: true
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Style.marginS
                        NButton {
                            text: "取消"
                            outlined: true
                            enabled: !root.busy
                            onClicked: root.closeAddAccount()
                        }
                        NButton {
                            Layout.fillWidth: true
                            text: "确认添加"
                            icon: "check"
                            enabled: !root.busy && root.apiKeyInput.trim() !== "" && (!root.relayMode || (root.relayDisplayName.trim() !== "" && root.relayBaseUrl.trim() !== ""))
                            onClicked: root.submitApiKey()
                        }
                    }
                }
            }
        }
    }

    Rectangle {
        id: proxyModal
        anchors.fill: parent
        visible: root.proxyOpen
        color: Qt.rgba(0, 0, 0, 0.42)
        z: 110

        MouseArea {
            anchors.fill: parent
            onClicked: root.closeProxy()
        }

        NBox {
            anchors.centerIn: parent
            width: Math.min(parent.width - 2 * Style.marginL, 520 * Style.uiScaleRatio)
            height: Math.min(parent.height - 2 * Style.marginL, proxyModalColumn.implicitHeight + 2 * Style.marginM)
            color: root.panelBackgroundColor

            Flickable {
                anchors.fill: parent
                anchors.margins: Style.marginM
                contentHeight: proxyModalColumn.implicitHeight
                clip: true

                ColumnLayout {
                    id: proxyModalColumn
                    width: parent.width
                    spacing: Style.marginM

                    RowLayout {
                        Layout.fillWidth: true
                        NText {
                            text: (root.selectedProxy ? root.selectedProxy.display_name : "Provider") + " · 本地代理"
                            font.weight: Style.fontWeightBold
                            color: Color.mOnSurface
                            Layout.fillWidth: true
                        }
                        NIconButton {
                            icon: "close"
                            tooltipText: "关闭"
                            enabled: !root.busy
                            onClicked: root.closeProxy()
                        }
                    }

                    NText {
                        text: (root.proxyRunning ? "运行中 · http://" : "已停止 · http://") + root.proxyListen + "/v1"
                        color: root.proxyRunning ? Color.mPrimary : Color.mOnSurfaceVariant
                        Layout.fillWidth: true
                        wrapMode: Text.WordWrap
                    }
                    NButton {
                        Layout.fillWidth: true
                        text: root.proxyRunning ? "停止代理" : "启动代理"
                        icon: root.proxyRunning ? "player-stop" : "player-play"
                        enabled: !root.busy
                        onClicked: root.toggleProxy()
                    }

                    NText {
                        text: "客户端凭证"
                        font.weight: Style.fontWeightSemiBold
                        color: Color.mOnSurface
                    }
                    Repeater {
                        model: root.proxyCredentials
                        RowLayout {
                            Layout.fillWidth: true
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 0
                                NText {
                                    text: modelData.display_name
                                    color: Color.mOnSurface
                                    Layout.fillWidth: true
                                    elide: Text.ElideRight
                                }
                                NText {
                                    text: modelData.token_prefix + "…"
                                    color: Color.mOnSurfaceVariant
                                    pointSize: Style.fontSizeXS
                                }
                            }
                            NButton {
                                visible: true
                                text: "撤销"
                                outlined: true
                                enabled: !root.busy
                                onClicked: root.disableProxyCredential(modelData.id)
                            }
                        }
                    }
                    NText {
                        visible: root.proxyCredentials.length === 0
                        text: "尚无客户端凭证；启动代理时会自动创建默认凭证。"
                        color: Color.mOnSurfaceVariant
                        pointSize: Style.fontSizeXS
                        wrapMode: Text.WordWrap
                        Layout.fillWidth: true
                    }

                    NTextInput {
                        Layout.fillWidth: true
                        placeholderText: "凭证名称"
                        text: root.proxyCredentialName
                        onTextChanged: root.proxyCredentialName = text
                    }
                    NTextInput {
                        Layout.fillWidth: true
                        placeholderText: "自定义 Token（可选，至少 24 字符）"
                        text: root.proxyCredentialToken
                        inputItem.echoMode: TextInput.Password
                        onTextChanged: root.proxyCredentialToken = text
                    }
                    NButton {
                        Layout.fillWidth: true
                        text: "创建客户端凭证"
                        icon: "key"
                        enabled: !root.busy && root.proxyCredentialName.trim() !== "" && (root.proxyCredentialToken.trim() === "" || root.proxyCredentialToken.trim().length >= 24)
                        onClicked: root.createProxyCredential()
                    }
                    NText {
                        visible: root.createdProxyToken !== ""
                        text: "新 Token（已复制到剪贴板，仅显示一次）：\n" + root.createdProxyToken
                        color: Color.mPrimary
                        wrapMode: Text.WrapAnywhere
                        Layout.fillWidth: true
                    }
                }
            }
        }
    }
}
