package mcmap.agent;

import java.io.FileOutputStream;
import java.io.OutputStream;
import java.io.PrintStream;
import java.lang.instrument.Instrumentation;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.net.InetAddress;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.time.Instant;
import java.util.HashMap;
import java.util.Locale;
import java.util.Map;

/**
 * MC Mapper's in-game agent: reads the local player's position and reports it to the map.
 *
 * Read-only. It transforms no bytecode, changes no game state and sends nothing to any server.
 * Every game name is looked up by reflection, so one jar serves every version until a name
 * changes, and a missing name is reported by name instead of crashing the game.
 *
 * Loaded either at run time (jcmd PID JVMTI.agent_load /path/mcmap-agent.jar port:47123,token:abc) or
 * at startup (-javaagent:/path/mcmap-agent.jar=port:47123,token:abc). Options are comma-separated
 * key:value pairs. jcmd parses "key=value" as its own named argument and drops it, so the separator
 * is a colon; "=" is accepted too for -javaagent.
 *   port:47123    the map's loopback port; omit to only log (the M0 spike)
 *   token:...     shared secret the map checks in the hello line
 *   log:/path     append one human-readable line per second here (default: a file next to the jar)
 *   hz:20         samples per second
 */
public final class Agent {
    private static volatile boolean started;

    public static void premain(String options, Instrumentation inst) {
        start(options, inst);
    }

    public static void agentmain(String options, Instrumentation inst) {
        start(options, inst);
    }

    private static synchronized void start(String options, Instrumentation inst) {
        if (started) {
            System.err.println("[mcmap-agent] already running; ignoring a second load");
            return;
        }
        started = true;
        Map<String, String> opts = parse(options);
        Thread t = new Thread(() -> new Agent(opts, inst).loop(), "mcmap-agent");
        t.setDaemon(true);
        t.setPriority(Thread.MIN_PRIORITY);
        t.start();
    }

    static Map<String, String> parse(String options) {
        Map<String, String> m = new HashMap<>();
        if (options == null) return m;
        for (String part : options.split(",")) {
            int colon = part.indexOf(':'), equals = part.indexOf('=');
            int eq = colon < 0 ? equals : equals < 0 ? colon : Math.min(colon, equals);
            if (eq > 0) m.put(part.substring(0, eq).trim(), part.substring(eq + 1).trim());
        }
        return m;
    }

    private final Map<String, String> opts;
    private final Instrumentation inst;
    private final PrintStream log;
    private final int port;
    private final String token;
    private final long periodMs;
    private Game game;
    private Socket socket;
    private OutputStream out;
    private long nextConnect;
    private long nextLog;
    private String lastStatus = "";

    private Agent(Map<String, String> opts, Instrumentation inst) {
        this.opts = opts;
        this.inst = inst;
        this.port = Integer.parseInt(opts.getOrDefault("port", "0"));
        this.token = opts.getOrDefault("token", "");
        int hz = Math.max(1, Math.min(60, Integer.parseInt(opts.getOrDefault("hz", "20"))));
        this.periodMs = 1000 / hz;
        this.log = openLog(opts.get("log"));
    }

    private static PrintStream openLog(String path) {
        try {
            if (path == null) {
                java.io.File jar = new java.io.File(Agent.class.getProtectionDomain().getCodeSource().getLocation().toURI());
                path = new java.io.File(jar.getParentFile(), "mcmap-agent.log").getPath();
            }
            return new PrintStream(new FileOutputStream(path, true), true, StandardCharsets.UTF_8);
        } catch (Exception e) {
            return System.err;
        }
    }

    private void say(String s) {
        String line = Instant.now() + " " + s;
        log.println(line);
        System.err.println("[mcmap-agent] " + s);
    }

    private void loop() {
        say("loaded, options " + redact(opts) + ", java " + System.getProperty("java.version"));
        while (true) {
            long t0 = System.currentTimeMillis();
            try {
                tick(t0);
            } catch (Throwable e) {
                status("error", e.toString());
                game = null;
            }
            long sleep = periodMs - (System.currentTimeMillis() - t0);
            if (sleep > 0) {
                try {
                    Thread.sleep(sleep);
                } catch (InterruptedException e) {
                    return;
                }
            }
        }
    }

    private void tick(long now) throws Exception {
        if (game == null) {
            Class<?> mc = find("net.minecraft.client.Minecraft");
            if (mc == null) {
                status("waiting", "net.minecraft.client.Minecraft is not loaded yet");
                Thread.sleep(1000);
                return;
            }
            game = new Game(mc);
            say("found the client: " + game.describe());
        }
        connect(now);
        Sample s = game.sample();
        if (s == null) {
            status("menu", null);
            return;
        }
        lastStatus = "";
        send(s.json());
        if (now >= nextLog) {
            nextLog = now + 1000;
            say(s.human());
        }
    }

    private Class<?> find(String name) {
        for (Class<?> c : inst.getAllLoadedClasses()) {
            if (c.getName().equals(name)) return c;
        }
        return null;
    }

    private void status(String state, String msg) {
        String key = state + msg;
        if (key.equals(lastStatus)) return;
        lastStatus = key;
        say("status " + state + (msg == null ? "" : ": " + msg));
        StringBuilder b = new StringBuilder("{\"v\":1,\"type\":\"status\",\"state\":").append(q(state));
        if (msg != null) b.append(",\"msg\":").append(q(msg));
        send(b.append('}').toString());
    }

    private void connect(long now) {
        if (port == 0 || out != null || now < nextConnect) return;
        nextConnect = now + 2000;
        try {
            Socket s = new Socket(InetAddress.getLoopbackAddress(), port);
            s.setTcpNoDelay(true);
            socket = s;
            out = s.getOutputStream();
            send("{\"v\":1,\"type\":\"hello\",\"token\":" + q(token) + ",\"agent\":\"mcmap-agent/0.1\",\"game\":" + q(game == null ? null : game.version()) + "}");
            say("connected to the map on port " + port);
        } catch (Exception e) {
            // The map isn't running; try again later, quietly.
        }
    }

    private void send(String line) {
        if (out == null) return;
        try {
            out.write((line + "\n").getBytes(StandardCharsets.UTF_8));
            out.flush();
        } catch (Exception e) {
            say("map connection closed: " + e.getMessage());
            try { socket.close(); } catch (Exception ignored) { }
            socket = null;
            out = null;
        }
    }

    static String q(String s) {
        if (s == null) return "null";
        StringBuilder b = new StringBuilder("\"");
        for (char c : s.toCharArray()) {
            switch (c) {
                case '"' -> b.append("\\\"");
                case '\\' -> b.append("\\\\");
                case '\n' -> b.append("\\n");
                case '\r' -> b.append("\\r");
                case '\t' -> b.append("\\t");
                default -> {
                    if (c < 0x20) b.append(String.format("\\u%04x", (int) c));
                    else b.append(c);
                }
            }
        }
        return b.append('"').toString();
    }

    private static String redact(Map<String, String> o) {
        Map<String, String> m = new HashMap<>(o);
        if (m.containsKey("token")) m.put("token", "***");
        return m.toString();
    }

    /** One reading of the player, taken on the client thread. */
    record Sample(long t, String dim, double x, double y, double z, float yaw, float pitch, String server, boolean realm, Long zoomSeed) {
        String json() {
            return String.format(Locale.ROOT,
                "{\"v\":1,\"type\":\"pos\",\"t\":%d,\"dim\":%s,\"x\":%.4f,\"y\":%.4f,\"z\":%.4f,\"yaw\":%.3f,\"pitch\":%.3f,\"server\":%s,\"realm\":%b,\"zoomSeed\":%s}",
                t, q(dim), x, y, z, yaw, pitch, q(server), realm, zoomSeed == null ? "null" : zoomSeed.toString());
        }

        String human() {
            return String.format(Locale.ROOT, "%s %.2f %.2f %.2f yaw %.1f pitch %.1f server %s%s zoomSeed %s",
                dim, x, y, z, yaw, pitch, server, realm ? " (realm)" : "", zoomSeed);
        }
    }

    /** The reflective handles, looked up once. Each lookup failure names what is missing. */
    static final class Game {
        final Method getInstance, execute, getCurrentServer, getX, getY, getZ, getYRot, getXRot, dimension, identifier, getBiomeManager;
        final Field player, level, serverName;
        final Method isRealm;
        final Field zoomSeed;
        final Method launchedVersion;

        Game(Class<?> mc) throws Exception {
            ClassLoader cl = mc.getClassLoader();
            getInstance = method(mc, "getInstance");
            execute = method(mc, "execute", Runnable.class);
            getCurrentServer = method(mc, "getCurrentServer");
            launchedVersion = optional(mc, "getLaunchedVersion");
            player = field(mc, "player");
            level = field(mc, "level");
            Class<?> entity = Class.forName("net.minecraft.world.entity.Entity", false, cl);
            getX = method(entity, "getX");
            getY = method(entity, "getY");
            getZ = method(entity, "getZ");
            getYRot = method(entity, "getYRot");
            getXRot = method(entity, "getXRot");
            Class<?> lvl = Class.forName("net.minecraft.world.level.Level", false, cl);
            dimension = method(lvl, "dimension");
            getBiomeManager = method(lvl, "getBiomeManager");
            identifier = method(Class.forName("net.minecraft.resources.ResourceKey", false, cl), "identifier");
            Class<?> serverData = Class.forName("net.minecraft.client.multiplayer.ServerData", false, cl);
            serverName = field(serverData, "name");
            isRealm = method(serverData, "isRealm");
            Field zs = Class.forName("net.minecraft.world.level.biome.BiomeManager", false, cl).getDeclaredField("biomeZoomSeed");
            zs.setAccessible(true);
            zoomSeed = zs;
        }

        String version() {
            try {
                return launchedVersion == null ? null : (String) launchedVersion.invoke(getInstance.invoke(null));
            } catch (Exception e) {
                return null;
            }
        }

        String describe() {
            return "version " + version();
        }

        /** Reads on the client thread and waits up to 250 ms; null when no world is loaded. */
        Sample sample() throws Exception {
            Object mc = getInstance.invoke(null);
            if (mc == null) return null;
            Sample[] box = new Sample[1];
            Exception[] err = new Exception[1];
            java.util.concurrent.CountDownLatch done = new java.util.concurrent.CountDownLatch(1);
            execute.invoke(mc, (Runnable) () -> {
                try {
                    box[0] = read(mc);
                } catch (Exception e) {
                    err[0] = e;
                } finally {
                    done.countDown();
                }
            });
            if (!done.await(250, java.util.concurrent.TimeUnit.MILLISECONDS)) return null;
            if (err[0] != null) throw err[0];
            return box[0];
        }

        private Sample read(Object mc) throws Exception {
            Object p = player.get(mc);
            Object l = level.get(mc);
            if (p == null || l == null) return null;
            String dim = identifier.invoke(dimension.invoke(l)).toString();
            Object sd = getCurrentServer.invoke(mc);
            String server = sd == null ? null : (String) serverName.get(sd);
            boolean realm = sd != null && (boolean) isRealm.invoke(sd);
            Object bm = getBiomeManager.invoke(l);
            Long seed = bm == null ? null : zoomSeed.getLong(bm);
            return new Sample(System.currentTimeMillis(), dim,
                (double) getX.invoke(p), (double) getY.invoke(p), (double) getZ.invoke(p),
                (float) getYRot.invoke(p), (float) getXRot.invoke(p), server, realm, seed);
        }

        private static Method method(Class<?> c, String name, Class<?>... args) throws Exception {
            try {
                return c.getMethod(name, args);
            } catch (NoSuchMethodException e) {
                throw new Exception("missing method " + c.getName() + "." + name + " (did the game rename it?)");
            }
        }

        private static Method optional(Class<?> c, String name) {
            try {
                return c.getMethod(name);
            } catch (NoSuchMethodException e) {
                return null;
            }
        }

        private static Field field(Class<?> c, String name) throws Exception {
            try {
                return c.getField(name);
            } catch (NoSuchFieldException e) {
                throw new Exception("missing field " + c.getName() + "." + name + " (did the game rename it?)");
            }
        }
    }
}
