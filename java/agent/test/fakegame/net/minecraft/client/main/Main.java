package net.minecraft.client.main;

/** Stand-in entry point with the real client's main class name, so `mcmap-cli spike` finds it. */
public class Main {
    public static void main(String[] args) throws Exception {
        net.minecraft.client.Minecraft.main(new String[] { "10000" });
    }
}
