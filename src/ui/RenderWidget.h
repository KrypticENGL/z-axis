#pragma once

#include "scene/Scene.h"

#include <QWidget>

#include <memory>

class IRenderer;

// Native child window the renderer presents into; Qt does no painting here.
class RenderWidget : public QWidget {
    Q_OBJECT
public:
    explicit RenderWidget(QWidget* parent = nullptr);
    ~RenderWidget() override;

    QPaintEngine* paintEngine() const override { return nullptr; }

    // Shown now if the renderer is up, otherwise once it initialises.
    void setScene(Scene scene);

protected:
    void showEvent(QShowEvent* event) override;
    void resizeEvent(QResizeEvent* event) override;
    void paintEvent(QPaintEvent* event) override;

private:
    QSize pixelSize() const;

    std::unique_ptr<IRenderer> m_renderer;
    Scene m_scene;
    bool m_ready = false;
};
