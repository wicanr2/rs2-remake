# RS-2 HD 工具映像：切圖、去背、輪廓量測。主機不裝任何 Python 套件。
FROM python@sha256:aa943ce411423c7582288f18707ff66eddcbaeb510502d747a72aeb34f30d159
RUN pip install --no-cache-dir Pillow==11.3.0 numpy==2.3.3
