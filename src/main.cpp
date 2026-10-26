#include "ui/RenderWidget.h"

#include <QApplication>
#include <QMainWindow>

int main(int argc, char* argv[])
{
    QApplication app(argc, argv);

    QMainWindow window;
    window.setWindowTitle("z-axis");
    window.setCentralWidget(new RenderWidget);
    window.resize(1280, 720);
    window.show();

    return app.exec();
}
