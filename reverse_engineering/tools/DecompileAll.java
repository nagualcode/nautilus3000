import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.listing.*;
import java.io.*;

public class DecompileAll extends GhidraScript {
    @Override
    public void run() throws Exception {
        String out = getScriptArgs()[0];
        DecompInterface d = new DecompInterface();
        DecompileOptions o = new DecompileOptions();
        d.setOptions(o);
        d.openProgram(currentProgram);
        PrintWriter pw = new PrintWriter(new FileWriter(out));
        FunctionIterator it = currentProgram.getFunctionManager().getFunctions(true);
        while (it.hasNext() && !monitor.isCancelled()) {
            Function f = it.next();
            if (f.isExternal() || f.isThunk()) continue;
            pw.println("/* ==== " + f.getName(true) + " @ " + f.getEntryPoint() + " ==== */");
            DecompileResults r = d.decompileFunction(f, 120, monitor);
            if (r != null && r.decompileCompleted()) pw.println(r.getDecompiledFunction().getC());
            else pw.println("/* decompile failed */");
        }
        pw.close();
    }
}
