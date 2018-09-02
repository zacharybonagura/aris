package edu.rpi.aris.assign;

import edu.rpi.aris.assign.spi.ArisModule;
import javafx.stage.Modality;
import javafx.stage.Window;

public interface ModuleUI<T extends ArisModule> {

    void show() throws Exception;

    void hide() throws Exception;

    void setModal(Modality modality, Window owner) throws Exception;

    void setDescription(String description) throws Exception;

    void setModuleUIListener(ModuleUIListener listener);

    Window getUIWindow();

    Problem<T> getProblem() throws Exception;

}
