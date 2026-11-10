package mcmap.agent;

/**
 * Checks every reflective name the agent uses against a client jar, without starting the game:
 *   java -cp mcmap-agent.jar:<client jar>:<libraries> mcmap.agent.SelfCheck
 * Prints "ok" or the first missing name.
 */
public final class SelfCheck {
    public static void main(String[] args) throws Exception {
        Class<?> mc = Class.forName("net.minecraft.client.Minecraft", false, SelfCheck.class.getClassLoader());
        new Agent.Game(mc);
        System.out.println("ok: every name the agent reads exists in this client");
    }
}
