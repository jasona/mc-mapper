package net.minecraft.client;

import java.util.concurrent.LinkedBlockingQueue;

/** A stand-in for the real client with the same public names, for testing the agent's attach. */
public class Minecraft {
    private static Minecraft instance;
    public net.minecraft.client.player.LocalPlayer player;
    public net.minecraft.client.multiplayer.ClientLevel level;
    private final LinkedBlockingQueue<Runnable> tasks = new LinkedBlockingQueue<>();

    public static Minecraft getInstance() { return instance; }
    public void execute(Runnable r) { tasks.add(r); }
    public String getLaunchedVersion() { return "fake-26.3"; }
    public net.minecraft.client.multiplayer.ServerData getCurrentServer() { return new net.minecraft.client.multiplayer.ServerData("Test Realm", true); }

    public static void main(String[] args) throws Exception {
        instance = new Minecraft();
        instance.player = new net.minecraft.client.player.LocalPlayer();
        instance.level = new net.minecraft.client.multiplayer.ClientLevel();
        System.out.println("fake game pid " + ProcessHandle.current().pid());
        long end = System.currentTimeMillis() + Long.parseLong(args.length > 0 ? args[0] : "15000");
        double t = 0;
        while (System.currentTimeMillis() < end) {
            Runnable r;
            while ((r = instance.tasks.poll()) != null) r.run();
            t += 0.05;
            instance.player.x = 2839 + 10 * Math.cos(t);
            instance.player.z = -1418 + 10 * Math.sin(t);
            Thread.sleep(50);
        }
    }
}
