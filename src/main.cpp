#include "scene/GltfLoader.h"
#include "ui/RenderWidget.h"

#include <QApplication>
#include <QMainWindow>

int main(int argc, char* argv[])
{
    QApplication app(argc, argv);

    const QStringList args = app.arguments();
    const QString path = args.size() > 1 ? args[1] : app.applicationDirPath() + "/assets/sample.glb";

    auto* view = new RenderWidget;
    Scene scene;
    std::string error;
    if (loadGltf(path.toStdString(), scene, error))
        view->setScene(std::move(scene));
    else
        qWarning("%s", error.c_str());

    QMainWindow window;
    window.setWindowTitle("z-axis");
    window.setCentralWidget(view);
    window.resize(1280, 720);
    window.show();

    return app.exec();
}
