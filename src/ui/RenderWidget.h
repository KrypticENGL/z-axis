#pragma once

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

protected:
    void showEvent(QShowEvent* event) override;
    void resizeEvent(QResizeEvent* event) override;
    void paintEvent(QPaintEvent* event) override;

private:
    QSize pixelSize() const;

    std::unique_ptr<IRenderer> m_renderer;
    bool m_ready = false;
};
